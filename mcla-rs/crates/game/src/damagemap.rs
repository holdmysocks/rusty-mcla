//! What a hit leaves on a car besides its damage points (docs/reference/vehicle/damage_map_rule.md):
//! the vertex damage map (`mcVertDamageTexture`, the draw object's +1020) and the 13 deform pieces
//! (+4656). Both are fed by the zone damage routine (0x82356D98, physics/car.rs `zone_hit` and
//! `scrape_hit`): a collision (kind 0) stamps a dent into the map's red channel at the hit's
//! direction from the car's origin, a scrape (kind 1) eight stamps into the blue channel along the
//! way the car came. The car's vertex programs move every vertex towards (0, 1.3, 0) of the car by
//! 0.2 m x red x green at its own direction (render/city.wgsl `vs_body`); the paint shows bare metal
//! and rubbed paint by the blue (`fs_main` mode 6). The pieces count hits near the wheel hubs (the
//! rims' scuff) and the windows (cracked, shattered).

use glam::Vec3;
use std::sync::OnceLock;

/// The map's side, in texels (0x8288D034 / 0x8288D038).
pub const SIZE: usize = 256;
/// The render targets the game has for the maps (0x8288D014[8]): a car made when all are taken has none.
pub const POOL: usize = 8;

/// One of the 13 deform pieces (80 bytes at the draw object's +4656): a sphere in the car that
/// counts the hits landing in it.
#[derive(Clone, Copy, Debug, Default)]
pub struct Piece {
    /// its centre in the car (+16) and radius (+48)
    pub local: Vec3,
    pub radius: f32,
    /// the collisions' sum (+52) and the scrapes' (+56)
    pub hits: f32,
    pub scrapes: f32,
    /// what a hit must exceed to count (+60, +64)
    pub hit_min: f32,
    pub scrape_min: f32,
}

/// A stamp waiting for the next draw (0x8230A180's 96-byte records: the direction, the amount).
#[derive(Clone, Copy, Debug)]
pub struct Stamp {
    pub dir: Vec3,
    pub amount: f32,
}

/// A scrape's sparks (0x82376408: `damage_scrape_sparks` at the point along the normal, moving
/// with the closing velocity, its evolution "impact" = min(10 x amount, 1)).
#[derive(Clone, Copy, Debug)]
pub struct Sparks {
    pub at: Vec3,
    pub normal: Vec3,
    pub rel: Vec3,
    pub amount: f32,
}

/// A car's share of the above: the pieces, this frame's stamps and effects, and the map's flags.
#[derive(Clone, Debug, Default)]
pub struct CarDamage {
    pub pieces: [Piece; 13],
    pub pieces_set: bool,
    /// collision stamps (red) and scrape stamps (blue) since the last draw: 8 of each at most,
    /// later ones are dropped (0x8230A180)
    pub dents: Vec<Stamp>,
    pub scrapes: Vec<Stamp>,
    /// the map has been stamped (+2154: the damage programs are drawn)
    pub stamped: bool,
    /// the respawn's repair (+2149): the dents go at the next draw, the scrapes stay
    pub repair: bool,
    /// the full reset (+2150, `sub_82308888` from `sub_82353890`): dents and scrapes go at the next draw
    pub reset: bool,
    /// a window's hit sum passed 1 or 2 (0x82376B78): the window's place in the car, numbered on
    /// (`glass_serial`) so the effect and the sound each take every one once; the last 8 kept
    pub glass: Vec<(u64, Vec3)>,
    pub glass_serial: u64,
    /// the last one the effects took (main.rs)
    pub glass_fx_seen: u64,
    pub sparks: Vec<Sparks>,
}

impl CarDamage {
    /// The pieces' places (0x8235CBD0): 0..3 the wheel hubs - radius 1, both thresholds 0.01;
    /// 4..7 at the origin with radius 0 (never hit); 8 front glass (0, mid y, min z), 9 rear glass
    /// (0, mid y, max z), radius 1; 10 left glass (min x, mid y, 0), 11 right glass (max x, mid y,
    /// 0), radius 2, thresholds 0; 12 unused. `lo` / `hi`: the car's bound box.
    pub fn set_up(&mut self, hubs: [Vec3; 4], lo: Vec3, hi: Vec3) {
        if self.pieces_set {
            return;
        }
        let mid = (hi.y + lo.y) * 0.5;
        let t = f32::from_bits(0x3c23_d70a);
        for (i, h) in hubs.iter().enumerate() {
            self.pieces[i] = Piece { local: *h, radius: 1.0, hit_min: t, scrape_min: t, ..Default::default() };
        }
        for (i, (at, r)) in [(Vec3::new(0.0, mid, lo.z), 1.0), (Vec3::new(0.0, mid, hi.z), 1.0), (Vec3::new(lo.x, mid, 0.0), 2.0), (Vec3::new(hi.x, mid, 0.0), 2.0)].into_iter().enumerate() {
            self.pieces[8 + i] = Piece { local: at, radius: r, ..Default::default() };
        }
        self.pieces_set = true;
    }

    /// A hit of `a` at `local` (in the car) on the pieces (0x82356F8C / 0x82351580): within a piece's
    /// radius, a scrape over its scrape threshold adds to its scrape sum, anything else over the hit
    /// threshold to its hit sum. A window whose hit sum passes 1 or 2 cracks / breaks (0x82376B78).
    pub fn pieces_hit(&mut self, local: Vec3, a: f32, kind: u8) {
        for i in 0..13 {
            let p = &mut self.pieces[i];
            let before = p.hits;
            // (F-63094: y squared first, then z and x fused into it, in double, each step rounded to a float)
            let (dx, dy, dz) = ((p.local.x - local.x) as f64, (p.local.y - local.y) as f64, (p.local.z - local.z) as f64);
            let dist = (dx.mul_add(dx, (dz.mul_add(dz, (dy * dy) as f32 as f64)) as f32 as f64) as f32 as f64).sqrt() as f32;
            if !(dist < p.radius) {
                continue;
            }
            if kind != 0 && a > p.scrape_min {
                p.scrapes += a;
            } else if a > p.hit_min {
                p.hits += a;
            }
            if (8..12).contains(&i) && ((before < 1.0 && p.hits >= 1.0) || (before < 2.0 && p.hits >= 2.0)) {
                // (the port keeps the last 8 for whoever plays them)
                if self.glass.len() >= 8 {
                    self.glass.remove(0);
                }
                self.glass_serial += 1;
                self.glass.push((self.glass_serial, p.local));
            }
        }
    }

    /// Queue a stamp (0x8230A180): 8 of a kind per draw at most.
    pub fn stamp(&mut self, kind: u8, dir: Vec3, amount: f32) {
        // (diagnostics MCLA_DBG_STAMPS=<file>: every queued stamp, appended - work/dmgmap/stamps_cmp.py)
        if let Some(p) = std::env::var_os("MCLA_DBG_STAMPS") {
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(p) {
                let _ = writeln!(f, "{kind} {:.6} {:.6} {:.6} {:.7} {}", dir.x, dir.y, dir.z, amount, if kind == 0 { self.dents.len() } else { self.scrapes.len() });
            }
        }
        self.stamped = true;
        let q = if kind == 0 { &mut self.dents } else { &mut self.scrapes };
        if q.len() < 8 {
            q.push(Stamp { dir, amount });
        }
    }

    /// The window i (0 front, 1 rear, 2 left, 3 right) as the glass draw has it (F-50822, `window_state`): 0
    /// whole, 1 cracked (a quarter of its hit sum at least 1), 2 broken (1.5).
    pub fn window(&self, i: usize) -> u8 {
        window_state(self.pieces[8 + i.min(3)].hits)
    }

    /// The four windows' hit sums (pieces 8 .. 11, +52), as F-50822 reads them.
    pub fn window_sums(&self) -> [f32; 4] {
        [0, 1, 2, 3].map(|i| self.pieces[8 + i].hits)
    }

    /// The four windows' states as the glass program takes them (WindowDmgVal slots 0 - 3).
    pub fn windows(&self) -> [f32; 4] {
        [0, 1, 2, 3].map(|i| self.window(i) as f32)
    }

    /// The wheel i's tyre dirt as the wheel draw has it (0x8235A8D0): its hub's scrape sum, at most
    /// 1 - TireDirtColor.w of the tyre program (handle +40 of the shader variables, `sub_82379270`).
    pub fn rim_scuff(&self, i: usize) -> f32 {
        self.pieces[i.min(3)].scrapes.clamp(0.0, 1.0)
    }

    /// The respawn's reset (0x82353B58): the pieces' sums to zero, the map's dents at the next draw.
    pub fn repair(&mut self) {
        for p in self.pieces.iter_mut() {
            p.hits = 0.0;
            p.scrapes = 0.0;
        }
        self.dents.clear();
        self.scrapes.clear();
        self.repair = true;
    }

    /// The full reset (mcCarModel vt[1] = 0x82353890 - a new car, the garage's `game.repairCar` through
    /// `sub_822A7388`): as the repair, and the map's scrapes cleared too.
    pub fn reset(&mut self) {
        self.repair();
        self.reset = true;
        // (F-50602 also clears +2154, the map's "has been stamped")
        self.stamped = false;
    }
}

/// F-50822's window state from a window piece's hit sum: s = 0.25 x sum; 2 unless s < 1.5, else 1 unless s < 1.0,
/// else 0 (fcmpu / blt: a NaN sum is broken).
pub fn window_state(sum: f32) -> u8 {
    let s = 0.25 * sum;
    if !(s < 1.5) {
        2
    } else if !(s < 1.0) {
        1
    } else {
        0
    }
}

/// The glass colours of a car's record that the glass draw hands on (P = car +132, from P + 64: + 224 the
/// headlight glass, + 240 the taillight glass, + 256 / + 272 the same for the lenses (bumped), + 288 the window
/// tint - the record's `m_vHeadlightGlassColorNew`, `m_vTaillightGlassColorNew`, `m_vBumpedHeadlightColor`,
/// `m_vBumpedTaillightColor`, `m_vWindowTintColorNew`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlassColours {
    pub head: [f32; 4],
    pub tail: [f32; 4],
    pub bumped_head: [f32; 4],
    pub bumped_tail: [f32; 4],
    pub tint: [f32; 4],
}

/// The glass a car's windscreen (zone 0) and zone 8 always take, whatever the tint (F-50822's image vector; the
/// record's constructor glass too).
pub const STOCK_GLASS: [f32; 4] = [0.0, 0.0035, 0.005, 0.372];

/// The glass colours of a car whose record the port does not have (traffic, parked, a player without a save): the
/// 100 % save's R8 record's (the headlight glass and its lens 0.392157, the taillight glass 0 and its lens 0.196078,
/// all black - what the capture of the original shows in TintColors / BumpedTintColors c209 + zone 4 - 7), the tint
/// the stock glass.
pub const RECORDED_GLASS: GlassColours = GlassColours {
    head: [0.0, 0.0, 0.0, 0.392157],
    tail: [0.0, 0.0, 0.0, 0.0],
    bumped_head: [0.0, 0.0, 0.0, 0.392157],
    bumped_tail: [0.0, 0.0, 0.0, 0.196078],
    tint: STOCK_GLASS,
};

/// What F-50822 hands the glass programs, per window zone 0 .. 8 (the zone a vertex carries): WindowDmgVal,
/// TintColors (the windows' program), BumpedTintColors (the lenses'), and ClipPlaneEq (gCarConstantBuffer +512).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlassVars {
    pub dmg: [f32; 9],
    pub tint: [[f32; 4]; 9],
    pub bumped: [[f32; 4]; 9],
    pub plane: [f32; 4],
}

/// F-50822 (checked against the original's code: `glass_vars_match_the_original`). `sums` the four windows' hit
/// sums; `in_car` G+123 (the camera's in-car view); `no_damage` G+6478 (the game's byte +435); `gate` (car+80)+41
/// (the damage gate the hood and trunk read); `bike` F-54299; `up` / `pos` rows 1 and 3 of the car's matrix; `h`
/// the model tune's fWindowClipPlaneHeight; `clip` the draw's flag (-1 in place of h without it).
/// - WindowDmgVal[i] = `window_state` for i < 4 outside the in-car view, else 0.
/// - TintColors[i]: 4, 5 the taillight glass; 6, 7 the headlight glass; 0, 8 `STOCK_GLASS` on a car (the tint on a
///   motorcycle); 1 .. 3 the window tint. BumpedTintColors the same with the bumped colours.
/// - i < 4 with the gate off, or with `no_damage` outside the in-car view: the state clamped to 0 .. 1 (fsel) and
///   TintColors[i].w = 1.
/// - the plane: n = up, p = up x h + pos (vmaddfp: product and sum rounded apart), w = -(p.z n.z + (p.y n.y + p.x
///   n.x)) fused (fmadds / fnmadds).
#[allow(clippy::too_many_arguments)]
pub fn glass_vars(sums: [f32; 4], in_car: bool, no_damage: bool, gate: bool, bike: bool, col: &GlassColours, up: [f32; 3], pos: [f32; 3], h: f32, clip: bool) -> GlassVars {
    let mut v = GlassVars { dmg: [0.0; 9], tint: [[0.0; 4]; 9], bumped: [[0.0; 4]; 9], plane: [0.0; 4] };
    for i in 0..9 {
        if i < 4 && !in_car {
            v.dmg[i] = window_state(sums[i]) as f32;
        }
        (v.tint[i], v.bumped[i]) = match i {
            4 | 5 => (col.tail, col.bumped_tail),
            6 | 7 => (col.head, col.bumped_head),
            0 | 8 if !bike => (STOCK_GLASS, STOCK_GLASS),
            _ => (col.tint, col.tint),
        };
        if i < 4 && (!gate || (no_damage && !in_car)) {
            let a = if v.dmg[i] >= 0.0 { v.dmg[i] } else { 0.0 };
            v.tint[i][3] = 1.0;
            v.dmg[i] = if a - 1.0 >= 0.0 { 1.0 } else { a };
        }
    }
    let x = if clip { h } else { -1.0 };
    let p = [0, 1, 2].map(|k| up[k] * x + pos[k]);
    let fma = |a: f32, b: f32, c: f32| (a as f64).mul_add(b as f64, c as f64) as f32;
    v.plane = [up[0], up[1], up[2], -fma(p[2], up[2], fma(p[1], up[1], p[0] * up[0]))];
    v
}

/// A stamp's shape: the red channel of `dmg_test_4.dds` (dents) or `dmg_test_3.dds` (scrapes).
pub struct Shape {
    pub w: usize,
    pub h: usize,
    pub r: Vec<f32>,
}

impl Shape {
    pub fn from_rgba(w: usize, h: usize, px: &[u8]) -> Shape {
        Shape { w, h, r: px.chunks_exact(4).map(|c| c[0] as f32 / 255.0).collect() }
    }

    /// Bilinear, clamped (the sampler's filter - V within 1 step in 100 texels of the AMD run, work/dmgmap; weights
    /// in 1/64 .. 1/512 steps change nothing)
    fn at(&self, u: f32, v: f32) -> f32 {
        let x = (u * self.w as f32 - 0.5).clamp(0.0, (self.w - 1) as f32);
        let y = (v * self.h as f32 - 0.5).clamp(0.0, (self.h - 1) as f32);
        let (x0, y0) = (x.floor() as usize, y.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(self.w - 1), (y0 + 1).min(self.h - 1));
        let (fx, fy) = (x - x0 as f32, y - y0 as f32);
        let r = |x: usize, y: usize| self.r[y * self.w + x];
        let a = r(x0, y0) + (r(x1, y0) - r(x0, y0)) * fx;
        let b = r(x0, y1) + (r(x1, y1) - r(x0, y1)) * fx;
        a + (b - a) * fy
    }
}

static SHAPES: OnceLock<(Shape, Shape)> = OnceLock::new();

/// The two stamp shapes from the disc (dent, scrape), set once at load (main.rs).
pub fn set_shapes(dent: Shape, scrape: Shape) {
    let _ = SHAPES.set((dent, scrape));
}

/// A car's map as the CPU keeps it, RGBA8 (R dents, G and A the car's layout, B scrapes), the texel (i, j)
/// read by the car's vertex programs for the direction (x, z) from the car's origin with
/// i = floor((x + 1) x 128), j = floor((z + 1) x 128); a stamp lands on it as at (i / 128 - 1, j / 128 - 1).
pub struct MapPixels {
    pub px: Vec<[u8; 4]>,
    /// the car's layout texture's green and alpha (T+2032, `<car>_maxdamage` of its .r83), what the resets put back
    pub layout: Vec<[u8; 2]>,
}

impl Default for MapPixels {
    fn default() -> Self {
        Self::new()
    }
}

impl MapPixels {
    /// The full reset (+2150) of a car without a layout texture: G and A 1 everywhere (I - every car on the disc
    /// checked has one) and no dents or scrapes.
    pub fn new() -> MapPixels {
        MapPixels { px: vec![[0, 255, 0, 255]; SIZE * SIZE], layout: vec![[255, 255]; SIZE * SIZE] }
    }

    /// The full reset (+2150) with the car's layout texture (raw RGBA8, `w` x `h`; its colour x (0, 1, 0, 1) drawn
    /// over the whole map under all four masks): (0, layout.g, 0, layout.a), the layout as the GPU reads it
    /// (`Texture::to_rgba8_gpu`). V, every texel after the reset (work/dmgmap): the R8 (256, linear), the Challenger
    /// '70 (256, gamma) and the city bus (32 x 32 RGBA8, stretched bilinearly).
    pub fn with_layout(layout: &[u8], w: usize, h: usize) -> MapPixels {
        if w == 0 || h == 0 || layout.len() != w * h * 4 {
            return MapPixels::new();
        }
        let at = |x: usize, y: usize, c: usize| layout[(y * w + x) * 4 + c] as f32;
        let layout: Vec<[u8; 2]> = (0..SIZE * SIZE)
            .map(|k| {
                let (i, j) = (k % SIZE, k / SIZE);
                if (w, h) == (SIZE, SIZE) {
                    return [at(i, j, 1) as u8, at(i, j, 3) as u8];
                }
                // (bilinear at the map's texel centres, the layout repeating - its fetch's address mode: V, the
                // city bus's 32 x 32 one, every texel (work/dmgmap))
                let x = (i as f32 + 0.5) * w as f32 / SIZE as f32 - 0.5;
                let y = (j as f32 + 0.5) * h as f32 / SIZE as f32 - 0.5;
                let (fx, fy) = (x - x.floor(), y - y.floor());
                let (x0, y0) = ((x.floor() as isize).rem_euclid(w as isize) as usize, (y.floor() as isize).rem_euclid(h as isize) as usize);
                let (x1, y1) = ((x0 + 1) % w, (y0 + 1) % h);
                let f = |c: usize| {
                    let a = at(x0, y0, c) + (at(x1, y0, c) - at(x0, y0, c)) * fx;
                    let b = at(x0, y1, c) + (at(x1, y1, c) - at(x0, y1, c)) * fx;
                    (a + (b - a) * fy).round().clamp(0.0, 255.0) as u8
                };
                [f(1), f(3)]
            })
            .collect();
        MapPixels { px: layout.iter().map(|l| [0, l[0], 0, l[1]]).collect(), layout }
    }

    /// The repair (+2149): red, green and alpha from the layout (mask R G A), the blue (scrapes) kept.
    pub fn repair(&mut self) {
        for (p, l) in self.px.iter_mut().zip(self.layout.iter()) {
            *p = [0, l[0], p[2], l[1]];
        }
    }

    /// The full reset (+2150, 0x82308888): red and blue to 0, green and alpha from the layout.
    pub fn reset(&mut self) {
        for (p, l) in self.px.iter_mut().zip(self.layout.iter()) {
            *p = [0, l[0], 0, l[1]];
        }
    }

    /// One stamp (0x82308F18, VertDamageBlit): a quad at c = (dir.x, dir.z) of half size `sx`
    /// across and `sy` x |c.xy| / |c| (c's third value -0.1) along the radius, its texture's red x
    /// trunc(amount x 255) / 255 added into `channel` (0 red, 2 blue), saturating at 1.
    pub fn stamp(&mut self, channel: usize, shape: &Shape, dir: Vec3, (sx, sy): (f32, f32), amount: f32) {
        let c = Vec3::new(dir.x, dir.z, -0.1);
        let nr = c.normalize();
        let t = Vec3::Z.cross(nr);
        let t = if t.length_squared() > 1e-12 { t.normalize() } else { Vec3::X };
        let (t2, n2) = ((t.x, t.y), (nr.x, nr.y));
        let n_len = (n2.0 * n2.0 + n2.1 * n2.1).sqrt();
        if n_len < 1e-6 {
            return;
        }
        let colour = (amount * 255.0).clamp(0.0, 255.0).trunc() / 255.0;
        if colour <= 0.0 {
            return;
        }
        // (the quad's corners, for the texels it can cover)
        let ext = sx * (t2.0.abs() + t2.1.abs()) + sy * (n2.0.abs() + n2.1.abs());
        // (texel i is drawn at map x = i / 128 - 1: D3D9's pixel centres, half a texel off the texel's middle - V,
        // work/dmgmap; u runs from the +t side, v from the -nr side: V against the original's maps)
        let to_i = |m: f32| ((m + 1.0) * 0.5 * SIZE as f32).floor();
        let (i0, i1) = (to_i(c.x - ext).max(0.0) as usize, (to_i(c.x + ext) + 1.0).clamp(0.0, (SIZE - 1) as f32) as usize);
        let (j0, j1) = (to_i(c.y - ext).max(0.0) as usize, (to_i(c.y + ext) + 1.0).clamp(0.0, (SIZE - 1) as f32) as usize);
        for j in j0..=j1 {
            for i in i0..=i1 {
                let (x, y) = (i as f32 / (SIZE / 2) as f32 - 1.0, j as f32 / (SIZE / 2) as f32 - 1.0);
                let (qx, qy) = (x - c.x, y - c.y);
                let u = (1.0 - (qx * t2.0 + qy * t2.1) / sx) * 0.5;
                let v = ((qx * n2.0 + qy * n2.1) / (n_len * n_len * sy) + 1.0) * 0.5;
                if !(0.0..=1.0).contains(&u) || !(0.0..=1.0).contains(&v) {
                    continue;
                }
                // (the blend's 8-bit step: rounded to the nearest - V, the original on an AMD GPU (work/dmgmap, s_amd:
                // 301 texels of 17975 differ); an NVIDIA GPU's blender steps from 0.50 .. 0.56 of a step instead
                // (blendtest), which is what the recordings on the RTX 3090 show)
                let add = (shape.at(u, v) * colour * 255.0 + 0.5).floor() as u32;
                let p = &mut self.px[j * SIZE + i][channel];
                *p = (*p as u32 + add).min(255) as u8;
            }
        }
    }

    /// Diagnostics: the map as a PNG (r dents, b scrapes; g and a shown as they are).
    pub fn dump(&self, path: &str) {
        let Ok(f) = std::fs::File::create(path) else { return };
        let mut enc = png::Encoder::new(std::io::BufWriter::new(f), SIZE as u32, SIZE as u32);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        if let Ok(mut w) = enc.write_header() {
            let _ = w.write_image_data(bytemuck::cast_slice(&self.px));
        }
    }

    /// The queued stamps into the map (0x82309238): dents into red at size 0.35 a + 0.25 with value
    /// a, scrapes into blue at size 0.4 with their value; the repair first. True if it changed.
    pub fn apply(&mut self, d: &mut CarDamage) -> bool {
        let mut changed = false;
        if d.repair {
            d.repair = false;
            if std::mem::take(&mut d.reset) {
                self.reset();
            } else {
                self.repair();
            }
            changed = true;
        }
        if let Some((dent, scrape)) = SHAPES.get() {
            for s in d.dents.iter() {
                let k = 0.35 * s.amount + 0.25;
                self.stamp(0, dent, s.dir, (k, k), s.amount);
                changed = true;
            }
            for s in d.scrapes.iter() {
                self.stamp(2, scrape, s.dir, (0.4, 0.4), s.amount);
                changed = true;
            }
        }
        d.dents.clear();
        d.scrapes.clear();
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// F-50822 against the original's code (tools/glass_vars_reference.py, testdata/glass_vars_ref.txt: 3,000 cases -
    /// window sums at and an ulp under the steps, NaN, infinite and negative, random record colours, car matrices
    /// with scaled up rows, clip heights, every in-car / no-damage / gate / motorcycle / flag / thread pick): every
    /// word of WindowDmgVal, TintColors, BumpedTintColors and the plane equal.
    #[test]
    fn glass_vars_match_the_original() {
        let f = |s: &str| f32::from_bits(u32::from_str_radix(s, 16).unwrap());
        let mut n = 0;
        for line in include_str!("testdata/glass_vars_ref.txt").lines() {
            let (i, o) = line.split_once(" -> ").unwrap();
            let iv: Vec<&str> = i.split_whitespace().collect();
            let w: Vec<f32> = iv[..57].iter().map(|x| f(x)).collect();
            let b: Vec<u32> = iv[57..].iter().map(|x| x.parse().unwrap()).collect();
            let ov: Vec<u32> = o.split_whitespace().map(|x| u32::from_str_radix(x, 16).unwrap()).collect();
            let r = |k: usize| [w[4 + 4 * k], w[5 + 4 * k], w[6 + 4 * k], w[7 + 4 * k]];
            let col = GlassColours { head: r(0), tail: r(1), bumped_head: r(2), bumped_tail: r(3), tint: r(4) };
            // (the matrix: +208 with the thread byte, else +144; rows 1 and 3)
            let m = if b[5] != 0 { &w[40..56] } else { &w[24..40] };
            let g = glass_vars([w[0], w[1], w[2], w[3]], b[0] != 0, b[1] != 0, b[2] != 0, b[3] != 0, &col, [m[4], m[5], m[6]], [m[12], m[13], m[14]], w[56], b[4] & 0xFF != 0);
            let mut got: Vec<u32> = g.dmg.iter().map(|x| x.to_bits()).collect();
            got.extend(g.tint.iter().flatten().map(|x| x.to_bits()));
            got.extend(g.bumped.iter().flatten().map(|x| x.to_bits()));
            got.extend(g.plane.iter().map(|x| x.to_bits()));
            assert_eq!(got, ov, "{line}");
            n += 1;
        }
        assert_eq!(n, 3000);
    }

    fn blob() -> Shape {
        let n = 32;
        let r = (0..n * n).map(|k| {
            let (x, y) = ((k % n) as f32 / (n - 1) as f32 * 2.0 - 1.0, (k / n) as f32 / (n - 1) as f32 * 2.0 - 1.0);
            (1.0 - (x * x + y * y).sqrt()).max(0.0)
        }).collect();
        Shape { w: n, h: n, r }
    }

    #[test]
    fn a_dent_lands_at_its_direction() {
        let mut m = MapPixels::new();
        // (a hit on the left side, level with the origin: direction (-1, 0.2, 0) - at the map's left edge, in the middle)
        let dir = Vec3::new(-1.0, 0.2, 0.0).normalize();
        m.stamp(0, &blob(), dir, (0.6, 0.6), 1.0);
        let at = |m: &MapPixels, x: f32, z: f32| {
            let i = (((x + 1.0) * 0.5) * SIZE as f32) as usize;
            let j = (((z + 1.0) * 0.5) * SIZE as f32) as usize;
            m.px[j.min(SIZE - 1) * SIZE + i.min(SIZE - 1)]
        };
        assert!(at(&m, dir.x, dir.z)[0] > 200, "{:?}", at(&m, dir.x, dir.z));
        assert_eq!(at(&m, 0.9, 0.0)[0], 0);
        assert_eq!(at(&m, dir.x, dir.z)[2], 0);
        // the repair takes the dent, keeps a scrape
        m.stamp(2, &blob(), dir, (0.4, 0.4), 0.5);
        let b = at(&m, dir.x, dir.z)[2];
        assert!(b > 100);
        m.repair();
        assert_eq!(at(&m, dir.x, dir.z), [0, 255, b, 255]);
    }

    /// The original's recorded stamp queues drawn by the port (work/dmgmap/cmpmap.py): MCLA_DMG_STAMPS = lines "D"
    /// (a draw) and "<kind> x y z a", MCLA_DMG_LAYOUT = the car's layout texture as raw RGBA8 256 x 256,
    /// MCLA_DMG_OUT = the map after them, raw RGBA8.
    #[test]
    #[ignore]
    fn replay_recorded_stamps() {
        let (Ok(stamps), Ok(out)) = (std::env::var("MCLA_DMG_STAMPS"), std::env::var("MCLA_DMG_OUT")) else { return };
        let shape = |name: &str| {
            let d = std::fs::read(format!("D:/MCLA/extracted/cache/textures/global/cars/globaltex/{name}")).unwrap();
            let t = mcla_formats::dds::parse_dds(name, &d).unwrap();
            Shape::from_rgba(t.width as usize, t.height as usize, &t.to_rgba8_gpu())
        };
        set_shapes(shape("dmg_test_4.dds"), shape("dmg_test_3.dds"));
        let mut m = match std::env::var("MCLA_DMG_LAYOUT").ok().and_then(|p| std::fs::read(p).ok()) {
            // (MCLA_DMG_LAYOUT_W: a square layout's side, 256 when not set)
            Some(l) => {
                let w = std::env::var("MCLA_DMG_LAYOUT_W").ok().and_then(|v| v.parse().ok()).unwrap_or(SIZE);
                MapPixels::with_layout(&l, w, w)
            }
            None => MapPixels::new(),
        };
        let mut d = CarDamage::default();
        for l in std::fs::read_to_string(stamps).unwrap().lines() {
            let v: Vec<&str> = l.split_whitespace().collect();
            if v.first() == Some(&"D") {
                m.apply(&mut d);
                continue;
            }
            let f = |i: usize| v[i].parse::<f32>().unwrap();
            d.stamp(v[0].parse().unwrap(), Vec3::new(f(1), f(2), f(3)), f(4));
        }
        m.apply(&mut d);
        std::fs::write(out, bytemuck::cast_slice::<[u8; 4], u8>(&m.px)).unwrap();
    }

    /// The quad's u runs from its +t side (verified against the original's maps, work/dmgmap): with a shape
    /// that grows with u, a stamp at the map's right edge (t = +z there) is strongest towards -z.
    #[test]
    fn a_stamp_is_oriented_as_the_original() {
        let n = 32;
        let ramp = Shape { w: n, h: n, r: (0..n * n).map(|k| (k % n) as f32 / (n - 1) as f32).collect() };
        let mut m = MapPixels::new();
        m.stamp(0, &ramp, Vec3::new(1.0, 0.2, 0.0).normalize(), (0.4, 0.4), 1.0);
        let col = |j: usize| (200..256).map(|i| m.px[j * SIZE + i][0] as u32).max().unwrap();
        assert!(col(128 - 40) > col(128 + 40) + 100, "{} {}", col(128 - 40), col(128 + 40));
        // (texel i lies at i / 128 - 1: a stamp centred on x = 0 is symmetric about texel 128, not 127.5)
        let mut m = MapPixels::new();
        m.stamp(0, &blob(), Vec3::new(0.0, 0.5, 1.0).normalize(), (0.2, 0.2), 1.0);
        let row = (0..SIZE).map(|j| m.px[j * SIZE + 128][0]).max().unwrap();
        assert!(row > 0);
        for d in 1..20 {
            let at = |i: usize| (0..SIZE).map(|j| m.px[j * SIZE + i][0]).max().unwrap();
            assert_eq!(at(128 - d), at(128 + d), "texel 128 -+ {d}");
        }
    }

    #[test]
    fn the_layout_comes_back_on_repair() {
        let mut l = vec![0u8; SIZE * SIZE * 4];
        for (k, p) in l.chunks_exact_mut(4).enumerate() {
            p.copy_from_slice(&[9, (k % 251) as u8, 9, (k % 13) as u8]);
        }
        let mut m = MapPixels::with_layout(&l, SIZE, SIZE);
        assert_eq!(m.px[1000], [0, (1000 % 251) as u8, 0, (1000 % 13) as u8]);
        m.px[1000] = [77, 1, 55, 2];
        m.repair();
        assert_eq!(m.px[1000], [0, (1000 % 251) as u8, 55, (1000 % 13) as u8]);
        // (the full reset - the garage's repairCar, a new car - takes the scrapes too)
        let mut d = CarDamage::default();
        d.reset();
        assert!(m.apply(&mut d) && !d.reset);
        assert_eq!(m.px[1000], [0, (1000 % 251) as u8, 0, (1000 % 13) as u8]);
        // (a 128 x 128 layout is spread over the 256 x 256 map)
        let small: Vec<u8> = (0..128 * 128).flat_map(|_| [0u8, 200, 0, 100]).collect();
        assert_eq!(MapPixels::with_layout(&small, 128, 128).px[SIZE * SIZE - 1], [0, 200, 0, 100]);
    }

    #[test]
    fn windows_and_hubs_count() {
        let mut d = CarDamage::default();
        d.set_up([Vec3::new(-0.8, 0.3, -1.3), Vec3::new(0.8, 0.3, -1.3), Vec3::new(-0.8, 0.3, 1.3), Vec3::new(0.8, 0.3, 1.3)], Vec3::new(-1.0, -0.5, -2.3), Vec3::new(1.0, 1.3, 2.1));
        // (the front glass at (0, 0.4, -2.3): four hits of 1 crack it, six break it)
        for k in 0..6 {
            d.pieces_hit(Vec3::new(0.0, 0.6, -2.0), 1.0, 0);
            assert_eq!(d.window(0), if k >= 5 { 2 } else if k >= 3 { 1 } else { 0 }, "after {}", k + 1);
        }
        assert_eq!(d.glass.len(), 2);
        // (the effect and the sound take them by number: the first at the sum 1, the second at 2)
        assert_eq!(d.glass.iter().map(|g| g.0).collect::<Vec<_>>(), vec![1, 2]);
        assert_eq!(d.windows(), [2.0, 0.0, 0.0, 0.0]);
        // (a scrape by the front left wheel scuffs its rim, not the glass)
        d.pieces_hit(Vec3::new(-1.0, 0.4, -1.2), 0.1, 1);
        assert!((d.rim_scuff(0) - 0.1).abs() < 1e-6 && d.rim_scuff(3) == 0.0);
        d.repair();
        assert_eq!((d.window(0), d.rim_scuff(0)), (0, 0.0));
    }
}
