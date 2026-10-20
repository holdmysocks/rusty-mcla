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
    /// the windows' places (car) where a window cracked or broke this tick: its effect and sound
    pub glass: Vec<Vec3>,
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
            if p.local.distance(local) >= p.radius {
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
                self.glass.push(p.local);
            }
        }
    }

    /// Queue a stamp (0x8230A180): 8 of a kind per draw at most.
    pub fn stamp(&mut self, kind: u8, dir: Vec3, amount: f32) {
        self.stamped = true;
        let q = if kind == 0 { &mut self.dents } else { &mut self.scrapes };
        if q.len() < 8 {
            q.push(Stamp { dir, amount });
        }
    }

    /// The window i (0 front, 1 rear, 2 left, 3 right) as the glass draw has it (0x82357DB8): 0
    /// whole, 1 cracked (a quarter of its hit sum at least 1), 2 broken (1.5).
    pub fn window(&self, i: usize) -> u8 {
        let s = 0.25 * self.pieces[8 + i.min(3)].hits;
        if s >= 1.5 { 2 } else if s >= 1.0 { 1 } else { 0 }
    }

    /// The wheel i's rim scuff as the wheel draw has it (0x8235A8D0): its hub's scrape sum, at most 1.
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

    /// Bilinear, clamped (the sampler's filter: I).
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

/// A car's map as the CPU keeps it, RGBA8, texel (i, j) the direction (x, z) = ((i + 0.5) / 128 - 1,
/// (j + 0.5) / 128 - 1) from the car's origin - the place the car's vertex programs read a vertex
/// of that direction at.
pub struct MapPixels {
    pub px: Vec<[u8; 4]>,
}

impl Default for MapPixels {
    fn default() -> Self {
        Self::new()
    }
}

impl MapPixels {
    /// The full reset (+2150): the layout - G and A 1 where the car has a surface, taken as 1
    /// everywhere (the per-car layout texture is not read: I) - and no dents or scrapes.
    pub fn new() -> MapPixels {
        MapPixels { px: vec![[0, 255, 0, 255]; SIZE * SIZE] }
    }

    /// The repair (+2149): red, green and alpha from the layout, the blue (scrapes) kept.
    pub fn repair(&mut self) {
        for p in self.px.iter_mut() {
            *p = [0, 255, p[2], 255];
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
        let to_i = |m: f32| ((m + 1.0) * 0.5 * SIZE as f32 - 0.5).floor();
        let (i0, i1) = (to_i(c.x - ext).max(0.0) as usize, (to_i(c.x + ext) + 1.0).clamp(0.0, (SIZE - 1) as f32) as usize);
        let (j0, j1) = (to_i(c.y - ext).max(0.0) as usize, (to_i(c.y + ext) + 1.0).clamp(0.0, (SIZE - 1) as f32) as usize);
        for j in j0..=j1 {
            for i in i0..=i1 {
                let (x, y) = ((i as f32 + 0.5) / (SIZE / 2) as f32 - 1.0, (j as f32 + 0.5) / (SIZE / 2) as f32 - 1.0);
                let (qx, qy) = (x - c.x, y - c.y);
                let u = ((qx * t2.0 + qy * t2.1) / sx + 1.0) * 0.5;
                let v = ((qx * n2.0 + qy * n2.1) / (n_len * n_len * sy) + 1.0) * 0.5;
                if !(0.0..=1.0).contains(&u) || !(0.0..=1.0).contains(&v) {
                    continue;
                }
                let add = (shape.at(u, v) * colour * 255.0).round() as u32;
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
            self.repair();
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
        // (a scrape by the front left wheel scuffs its rim, not the glass)
        d.pieces_hit(Vec3::new(-1.0, 0.4, -1.2), 0.1, 1);
        assert!((d.rim_scuff(0) - 0.1).abs() < 1e-6 && d.rim_scuff(3) == 0.0);
        d.repair();
        assert_eq!((d.window(0), d.rim_scuff(0)), (0, 0.0));
    }
}
