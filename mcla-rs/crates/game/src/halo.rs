//! The lamps' glow pass that the original draws after its tone map (LIGHTING_WORK.md "The
//! lamps' glow pass after the tone map"): a "seed" quad and three "star" quads per light, in
//! display terms, added to the frame before the display ramp (render/halo.wgsl; the tone map
//! in post.wgsl adds the halo picture to its encoded value).
//!
//! The rule is the game's `AddGlow` (F-92524) with the numbers of
//! tune/lighting/GlowTuning.xml; every array of that file has four entries, one a weather
//! (nice, cloudy, stormy, foggy - read from the running game while the weather was switched).

use crate::render::Gpu;
use glam::Vec3;
use std::sync::Mutex;

/// most glows a frame (the game's list holds 96)
pub const MAX: usize = 96;

/// tune/lighting/GlowTuning.xml, the entries the pass uses
#[derive(Clone, Debug)]
pub struct Tune {
    /// sm_fPropGlowIntensitys
    pub prop_intensity: [f32; 4],
    /// sm_PropGlowSeedSizes
    pub prop_size: [[f32; 2]; 4],
    /// sm_fSignalGlowSizes / sm_fSignalGlowIntensitys
    pub signal_size: [f32; 4],
    pub signal_intensity: [f32; 4],
    /// sm_fSignalGlowReflectionSizes (vec2 a weather; the road reflection's rectangle, siglrefl.rs)
    pub signal_refl: [[f32; 2]; 4],
    /// sm_fCarGlowIntensitys / sm_CarLightGlowSizess
    pub car_intensity: [f32; 4],
    pub car_size: [[f32; 2]; 4],
    /// sm_fStreakIntensitys / sm_fBloomIntensitys (the star's and the seed's factor): the item in use
    /// (`night_item`), a weather each
    pub streak: [f32; 4],
    pub bloom: [f32; 4],
    /// the files' two items of those (0 by day, 1 at night: the light manager's weather sum, F-70200,
    /// takes item IsNight)
    pub streak_items: [[f32; 4]; 2],
    pub bloom_items: [[f32; 4]; 2],
    /// StarSize
    pub star: [[f32; 2]; 4],
    /// sm_fPropTrailIntensitys / sm_fCarTrailIntensitys (the light trails' strength: trail.rs)
    pub prop_trail: [f32; 4],
    pub car_trail: [f32; 4],
    /// sm_PropGlowReflectionSeedSize / sm_fPropGlowReflectionIntensity: the prop lamps' rectangles in the
    /// road's reflection picture (F-28456's reflection branch, siglrefl.rs)
    pub prop_refl_size: [f32; 2],
    pub prop_refl_intensity: f32,
    /// sm_CarLightGlowReflectionSizess / sm_fCarGlowReflectionIntensity: the car lamps' rectangles in the
    /// road's reflection picture (F-96521's reflection branch, siglrefl.rs)
    pub car_refl_size: [[f32; 2]; 4],
    pub car_refl_intensity: f32,
    /// sm_fGlobalNightLightIntensity: gPropNFLColor.w (F-75018) and gPropNFLColorVS.w (F-40496) are m_NFLFade x this
    /// while IsNight (F-47292: hour > 18 or < 6, never under the garage's lighting), m_NFLFade alone otherwise;
    /// ShaderGlobals.w is 2 x this (F-45287)
    pub night_light: f32,
    /// MaxGlowDist, MaxGlowBias, MaxCarGlowBias, MinGlowBias
    pub max_dist: f32,
    pub max_bias: f32,
    pub max_car_bias: f32,
    pub min_bias: f32,
    /// RainGlowIntensity (+36, per weather): the factor of a glow's strength in the rain's list (F-92524)
    pub rain_glow: [f32; 4],
}

impl Default for Tune {
    fn default() -> Self {
        Tune {
            prop_intensity: [4.35, 4.3, 4.3, 2.0],
            prop_size: [[27.0, 13.8], [27.0, 14.0], [25.4, 9.8], [23.1875, 14.9]],
            signal_size: [4.52, 5.25, 8.7675, 10.0],
            signal_intensity: [4.52, 5.0, 5.0, 3.0],
            signal_refl: [[5.0, 5.0]; 4],
            car_intensity: [5.35, 5.3, 5.3, 5.3],
            car_size: [[6.0, 2.6]; 4],
            streak: [0.15; 4],
            bloom: [1.0, 1.0, 1.0, 2.0],
            streak_items: [[0.15; 4]; 2],
            bloom_items: [[1.0, 1.0, 1.0, 2.0]; 2],
            star: [[6.6, 3.85], [6.6, 3.85], [6.6, 3.85], [6.6, 3.2]],
            prop_trail: [0.5, 0.5, 0.55, 0.5],
            car_trail: [0.5; 4],
            prop_refl_size: [100.0, 100.0],
            prop_refl_intensity: 3.8,
            car_refl_size: [[0.0, 0.0], [0.0, 0.0], [20.0, 30.0], [0.0, 0.0]],
            car_refl_intensity: 1.55,
            night_light: 1.5,
            max_dist: 0.6,
            max_bias: 0.05,
            max_car_bias: 0.02,
            min_bias: 0.009,
            rain_glow: [0.05; 4],
        }
    }
}

/// The numbers between `<name ...>` and `</name>` (the first such element).
fn floats(text: &str, name: &str) -> Vec<f32> {
    let Some(i) = text.find(&format!("<{name}")) else { return Vec::new() };
    let rest = &text[i..];
    let (Some(a), Some(b)) = (rest.find('>'), rest.find(&format!("</{name}>"))) else {
        // `<name value="..."/>`
        let end = rest.find("/>").unwrap_or(0);
        return rest[..end].split('"').nth(1).and_then(|v| v.trim().parse().ok()).into_iter().collect();
    };
    if b < a {
        return Vec::new();
    }
    // (nested `<Item ...>` tags: keep what parses as a number)
    rest[a + 1..b].split(|c: char| c.is_whitespace() || c == '<' || c == '>').filter_map(|v| v.parse().ok()).collect()
}

impl Tune {
    pub fn parse(text: &str) -> Tune {
        let mut t = Tune::default();
        let four = |name: &str, d: [f32; 4]| {
            let v = floats(text, name);
            if v.len() >= 4 { [v[0], v[1], v[2], v[3]] } else { d }
        };
        let pairs = |name: &str, d: [[f32; 2]; 4]| {
            let v = floats(text, name);
            if v.len() >= 8 { [[v[0], v[1]], [v[2], v[3]], [v[4], v[5]], [v[6], v[7]]] } else { d }
        };
        let one = |name: &str, d: f32| floats(text, name).first().copied().unwrap_or(d);
        t.prop_intensity = four("sm_fPropGlowIntensitys", t.prop_intensity);
        t.prop_size = pairs("sm_PropGlowSeedSizes", t.prop_size);
        t.signal_size = four("sm_fSignalGlowSizes", t.signal_size);
        t.signal_intensity = four("sm_fSignalGlowIntensitys", t.signal_intensity);
        t.signal_refl = pairs("sm_fSignalGlowReflectionSizes", t.signal_refl);
        t.car_intensity = four("sm_fCarGlowIntensitys", t.car_intensity);
        t.car_size = pairs("sm_CarLightGlowSizess", t.car_size);
        // (two `<Item>`s: day, night; a file with one item uses it for both)
        let items = |name: &str, d: [[f32; 4]; 2]| {
            let v = floats(text, name);
            match v.len() {
                n if n >= 8 => [[v[0], v[1], v[2], v[3]], [v[4], v[5], v[6], v[7]]],
                n if n >= 4 => [[v[0], v[1], v[2], v[3]]; 2],
                _ => d,
            }
        };
        t.streak_items = items("sm_fStreakIntensitys", t.streak_items);
        t.bloom_items = items("sm_fBloomIntensitys", t.bloom_items);
        t.streak = t.streak_items[0];
        t.bloom = t.bloom_items[0];
        t.star = pairs("StarSize", t.star);
        t.prop_trail = four("sm_fPropTrailIntensitys", t.prop_trail);
        t.car_trail = four("sm_fCarTrailIntensitys", t.car_trail);
        // (`<name x=".." y=".."/>`)
        let xy = |name: &str, d: [f32; 2]| -> [f32; 2] {
            let Some(i) = text.find(&format!("<{name} ")) else { return d };
            let tag = &text[i..i + text[i..].find('>').unwrap_or(0)];
            let at = |k: &str| tag.find(&format!(" {k}=\"")).and_then(|j| tag[j + k.len() + 3..].split('"').next()?.trim().parse().ok());
            [at("x").unwrap_or(d[0]), at("y").unwrap_or(d[1])]
        };
        t.prop_refl_size = xy("sm_PropGlowReflectionSeedSize", t.prop_refl_size);
        t.prop_refl_intensity = one("sm_fPropGlowReflectionIntensity", t.prop_refl_intensity);
        t.car_refl_size = pairs("sm_CarLightGlowReflectionSizess", t.car_refl_size);
        t.car_refl_intensity = one("sm_fCarGlowReflectionIntensity", t.car_refl_intensity);
        t.night_light = one("sm_fGlobalNightLightIntensity", t.night_light);
        t.max_dist = one("MaxGlowDist", t.max_dist);
        t.max_bias = one("MaxGlowBias", t.max_bias);
        t.max_car_bias = one("MaxCarGlowBias", t.max_car_bias);
        t.min_bias = one("MinGlowBias", t.min_bias);
        t.rain_glow = four("RainGlowIntensity", t.rain_glow);
        t
    }

    /// The streak / bloom item the light manager's weather sum (F-70200) reads: the
    /// nested arrays' item 1 when IsNight (hour past the manager's +14568 = 18 or before its +14564 = 6,
    /// not in the garage), else item 0. (The disc's file has equal items.)
    pub fn night_item(&mut self, night: bool) {
        let i = night as usize;
        self.streak = self.streak_items[i];
        self.bloom = self.bloom_items[i];
    }

    /// F-70200's weather sum over every per-weather member it reads (sm_PropGlowSeedSizes,
    /// sm_fPropGlowIntensitys, sm_fPropTrailIntensitys, sm_fCarGlowIntensitys, sm_fCarTrailIntensitys,
    /// the streak / bloom items, sm_fSignalGlowSizes, sm_fSignalGlowReflectionSizes,
    /// sm_fSignalGlowIntensitys, sm_CarLightGlowSizess, and StarSize through F-86405): weight x
    /// member added over the weathers, the consumers (AddGlow F-92524) reading the sums. With the
    /// port's two-weather cross-fade (weights 1 - f and f) that is entry `w` moved towards entry `nw`
    /// by `f`; the sums are written into entry `w`, which the glow functions are then called with.
    pub fn blend(&mut self, w: usize, nw: usize, f: f32) {
        fn l1(a: &mut [f32; 4], w: usize, nw: usize, f: f32) {
            a[w] += (a[nw] - a[w]) * f;
        }
        fn l2(a: &mut [[f32; 2]; 4], w: usize, nw: usize, f: f32) {
            for k in 0..2 {
                a[w][k] += (a[nw][k] - a[w][k]) * f;
            }
        }
        if w == nw || w > 3 || nw > 3 {
            return;
        }
        for a in [&mut self.prop_intensity, &mut self.signal_size, &mut self.signal_intensity, &mut self.car_intensity, &mut self.streak, &mut self.bloom, &mut self.prop_trail, &mut self.car_trail, &mut self.rain_glow] {
            l1(a, w, nw, f);
        }
        for a in [&mut self.prop_size, &mut self.signal_refl, &mut self.car_size, &mut self.car_refl_size, &mut self.star] {
            l2(a, w, nw, f);
        }
    }
}

/// Whether a prop's lamp is lit at this hour (F-28456): after 17.9 - o and before 6.1 + o
/// o'clock, o = one of sixteen steps of a minute a lamp (the original takes bits 4..7 of the
/// prop's address x 1 / 960 h; here a hash of the lamp's place).
pub fn lamp_lit(pos: Vec3, hour: f32) -> bool {
    let o = ((pos.x.to_bits() ^ pos.z.to_bits().rotate_left(13)).wrapping_mul(0x9E37_79B1) >> 28) as f32 / 60.0;
    hour > 17.9 - o || hour < 6.1 + o
}

/// Which props are in the frame's list of lamp and signal props (the prop manager's third
/// per-view list, filled by F-79256 through the callback F-15084 and read by
/// F-28456 -> AddGlow): the game walks its 40 m prop grid in a square spiral from the MAIN
/// camera's cell and takes a prop when, with R = its drawable's sphere radius (1.0 for a lamp
/// or signal prop under 0.15) and dd = max(distance from the camera to its box centre - R, 0):
///   dd <= 200 and dd < 80 R / tan(fov / 2) and (its box's sphere touches the view's frustum
///   or dd < 10 R / tan(fov / 2))
///   or: dd < 250 and dd < R / (0.0125 tan 67.5 deg) = 33.14 R and the occlusion query passes.
/// No other test (checked on the running game: 741 props round the gate, 103 in the list, all
/// explained - work/lighting/lm/rule.py).
pub struct ListView {
    cam: Vec3,
    tan: f32,
    /// the main view's planes (F-53768: the frame's frustum)
    frustum: crate::render::Frustum,
    cell: (i32, i32),
    spiral: Vec<u16>,
}

/// the prop grid: cell 40 m, the first cell's centre (read from the running game: grid + 48)
const GRID: (f32, f32, f32) = (40.0, -3470.89, -2330.4475);
const SPIRAL_R: i32 = 9;

impl ListView {
    /// `frustum`: the main view's (the frame's `Frustum`, F-53768 from its view-projection).
    pub fn new(frustum: crate::render::Frustum, cam: Vec3, fov_y: f32) -> ListView {
        // (the spiral of F-44865: z up, x up, z down, x down, a ring wider each round)
        let n = (2 * SPIRAL_R + 1) as usize;
        let mut spiral = vec![u16::MAX; n * n];
        let (mut x, mut z, mut along_x, mut plus) = (0i32, 0i32, false, true);
        let (mut xmax, mut xmin, mut zmax, mut zmin) = (1, -1, 1, -1);
        for k in 0..(n * n) as u16 {
            if x.abs() > SPIRAL_R || z.abs() > SPIRAL_R {
                break;
            }
            spiral[((z + SPIRAL_R) as usize) * n + (x + SPIRAL_R) as usize] = k;
            match (along_x, plus) {
                (false, true) => { z += 1; if z == zmax { along_x = true; } }
                (false, false) => { z -= 1; if z == zmin { along_x = true; zmax += 1; zmin -= 1; } }
                (true, true) => { x += 1; if x == xmax { along_x = false; plus = false; } }
                (true, false) => { x -= 1; if x == xmin { along_x = false; plus = true; xmax += 1; xmin -= 1; } }
            }
        }
        let cell = (((cam.x - GRID.1) / GRID.0).round() as i32, ((cam.z - GRID.2) / GRID.0).round() as i32);
        ListView { cam, tan: (fov_y * 0.5).tan(), frustum, cell, spiral }
    }

    /// Whether a prop is on the lamp and signal list (F-79256's flag 0x2, `visit`): `kind` the object's type
    /// (4 lamp props, 5 signal props), `centre`, `w` its sphere (+96, the box centre and half diagonal), `r` its
    /// drawable's radius, `gfx` its drawable's byte +308 (its type's name has `_gfx_`, set by the type loader F-50983);
    /// `hidden`: the occlusion test (F-38575 on the sphere), asked only when the visit asks it.
    pub fn holds(&self, kind: i32, centre: Vec3, w: f32, r: f32, gfx: bool, hidden: impl FnOnce() -> bool) -> bool {
        let k = [0.0125 * self.tan, 0.1 * self.tan, 0.030_177_67];
        // (the view's sphere test F-29884 on the main view's 8 plane rows)
        let vis = || self.frustum.sphere_touches(centre, w);
        // (+156 is the prop's array of light places: set for every prop that has glows, the only ones asked here;
        // the far global's +8 is 3000, read from the running game - it moves only the value and flag 0x1)
        let v = visit(self.cam, k, 3000.0, kind, gfx as u8, true, r, (centre, w), vis, || if hidden() { 0x1000_0000 } else { 0x2000_0000 });
        v.map_or(false, |v| v.flags & 0x2 != 0)
    }

    /// A sphere inside the main view's planes (F-91781: every plane's value + r >= 0).
    pub fn sphere_in_view(&self, centre: Vec3, r: f32) -> bool {
        self.frustum.sphere_plus(centre, r)
    }

    /// The place of a prop's cell in the walk (the list's order; AddGlow keeps the first 96).
    pub fn order(&self, origin: Vec3) -> u32 {
        let (x, z) = (((origin.x - GRID.1) / GRID.0).round() as i32 - self.cell.0, ((origin.z - GRID.2) / GRID.0).round() as i32 - self.cell.1);
        if x.abs() > SPIRAL_R || z.abs() > SPIRAL_R {
            return u32::MAX;
        }
        self.spiral[((z + SPIRAL_R) * (2 * SPIRAL_R + 1) + x + SPIRAL_R) as usize] as u32
    }
}

/// What F-79256 hands the list's callback for one object (the manager's +116 / +120 / +124).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Visit {
    /// 0x1 the main view's list, 0x2 the lamp and signal list (also set with 0x30), 0x30 the far list
    pub flags: u32,
    /// clamp(R / k1 - R / 2, 0, far x 0.75)
    pub value: f32,
    /// max(distance - R, 0)
    pub dist: f32,
}

/// F-79256 for one object of the list (its node flagged 0x8000): `cam` the query's camera (+176), `k` its three
/// size factors (+192 / +196 / +200: 0.0125 tan(fov / 2), 0.1 tan(fov / 2), 0.0125 tan 67.5 deg), `far` the global's
/// +8, `kind` the object's type (vfunc 1), `skip` its drawable's byte +308 (a lamp prop with 1 is passed over),
/// `w156` its word +156, `radius` its drawable's +28 (under 0.15: 1.0 for a lamp or signal prop with +156
/// set), `vis` the view's sphere test (F-29884), `occ` the occlusion's answer (F-38575, mode 2: 0x10000000
/// hidden, 0x04000000 outside). Distance: dz^2, then dy and dx fused in, a single sqrt. None: not listed.
/// Checked against the original's code run offline.
#[allow(clippy::too_many_arguments)]
pub fn visit(cam: Vec3, k: [f32; 3], far: f32, kind: i32, skip: u8, w156: bool, radius: f32, sphere: (Vec3, f32),
             vis: impl FnOnce() -> bool, occ: impl FnOnce() -> u32) -> Option<Visit> {
    if kind == 4 && skip == 1 {
        return None;
    }
    let cap = far * 0.75;
    let mut r = radius;
    if r < 0.15 && (kind == 4 || kind == 5) && w156 {
        r = 1.0;
    }
    let c = sphere.0;
    let (dx, dy, dz) = (cam.x - c.x, cam.y - c.y, cam.z - c.z);
    let d = dx.mul_add(dx, dy.mul_add(dy, dz * dz)).sqrt() - r;
    let dd = if d >= 0.0 { d } else { 0.0 };
    let (mut main, mut lamp, mut far_l) = (false, false, false);
    if !(k[0] * dd >= r) {
        if vis() {
            main = dd <= cap;
            lamp = dd <= 200.0;
        } else if !(k[1] * dd >= r) {
            lamp = dd <= 200.0;
        }
    }
    if !(k[2] * dd >= r) {
        far_l = dd < 250.0;
    }
    let q = r / k[0];
    let v = -(r * 0.5 - q);
    let v = if v >= 0.0 { v } else { 0.0 };
    let value = if v - cap >= 0.0 { cap } else { v };
    if main || far_l {
        let o = occ();
        let ok = o & 0x0400_0000 == 0 && o & 0x1000_0000 == 0;
        main &= ok;
        far_l &= ok;
    }
    if !(main || lamp || far_l) {
        return None;
    }
    Some(Visit { flags: if far_l { 0x32 } else { 0 } | if lamp { 0x2 } else { 0 } | main as u32, value, dist: dd })
}

/// One light of the pass as the shader takes it (halo.wgsl `vs_halo`).
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Glow {
    /// the place (pulled towards the camera), a = the seed's strength
    pub pos_a: [f32; 4],
    /// the colour, a = the star's strength
    pub col_s: [f32; 4],
    /// the seed's half size (m), the star's long and short half diagonal (m)
    pub size: [f32; 4],
}

/// The vertex colour's alpha byte as the game writes it: min(factor x intensity, 10) x 25.5,
/// cut to a whole number; nothing under 0.001.
fn alpha(v: f32) -> f32 {
    if v < 0.001 { 0.0 } else { (v.min(10.0) * 25.5).floor().min(255.0) / 255.0 }
}

/// A glow's colour as AddGlow (F-92524) takes it: one packed word, made by its callers with F-59032 -
/// each channel clamped to 0 .. 1, x 255 and cut to a whole byte (a car's red 3.73 is 255, its green
/// 0.119 is 30). The draw, the trail and the rain's list read the bytes back as k / 255. (Packing k / 255
/// again gives k for every byte, so a colour already in bytes passes as it is.)
pub fn pack(c: [f32; 3]) -> [f32; 3] {
    c.map(|v| ((v.clamp(0.0, 1.0) * 255.0) as u8) as f32 / 255.0)
}

impl Tune {
    /// What `AddGlow` makes of a light: `size` and `intensity` as its caller hands them over,
    /// `car` = its flag for the lights of cars and signals (the size grows with distance so
    /// they stay visible, and the pull towards the camera goes by other numbers).
    fn glow(&self, w: usize, pos: Vec3, color: [f32; 3], mut size: [f32; 2], intensity: f32, car: bool, cam: Vec3, tan_half_fov: f32) -> Option<Glow> {
        let color = pack(color);
        let d = pos.distance(cam);
        let (seed, star) = (alpha(self.bloom[w] * intensity), alpha(self.streak[w] * intensity));
        let (mut hi, lo, reach) = if car { (self.max_car_bias * 0.5, 0.065, 10.0) } else { (self.max_bias, self.min_bias, self.max_dist) };
        if car {
            let k = d * tan_half_fov * (0.01 + 0.29 * (d / 400.0).clamp(0.0, 1.0));
            size = [size[0] + k, size[1] + k];
        } else if pos.distance_squared(Vec3::new(865.0, 23.0, -806.0)) < 169.0 || pos.distance_squared(Vec3::new(-3097.0, -15.4, 184.0)) < 169.0 {
            // (two spots where the game keeps the pull short)
            hi = 0.01;
        }
        let f = d.min(reach) / reach.max(1e-6);
        let bias = (f * f * (hi - lo) + lo) * (5.0 / 3.0);
        let p = pos + (cam - pos) * bias;
        rain_add(cam - pos, p, intensity * self.rain_glow[w], color);
        if seed <= 0.0 && star <= 0.0 {
            return None;
        }
        Some(Glow {
            pos_a: [p.x, p.y, p.z, seed],
            col_s: [color[0], color[1], color[2], star],
            size: [size[0] * 0.1, size[1] * 0.1, size[0] * self.star[w][0] * 0.45 * 0.05, size[1] * self.star[w][1] * 0.45 * 0.05],
        })
    }

    /// A lit prop lamp (street light, gate lamp ...).
    pub fn prop(&self, w: usize, pos: Vec3, color: Vec3, cam: Vec3) -> Option<Glow> {
        self.glow(w, pos, [color.x, color.y, color.z], self.prop_size[w], self.prop_intensity[w], false, cam, 0.0)
    }

    /// A car's lit lamp (F-96521): `facing` the way the lamp looks, `k` its level x the
    /// kind's factor; the strength goes by how squarely it faces the camera.
    pub fn car(&self, w: usize, pos: Vec3, facing: Vec3, color: [f32; 3], k: f32, cam: Vec3, tan_half_fov: f32) -> Option<Glow> {
        let i = k * (cam - pos).normalize_or_zero().dot(facing).clamp(0.0, 1.0);
        if i <= 0.001 {
            return None;
        }
        self.glow(w, pos, color, self.car_size[w], self.car_intensity[w] * i, true, cam, tan_half_fov)
    }

    /// A police car's light bar lamp (F-73214): its own AddGlow call with the props' pull towards the
    /// camera (not the cars'), size 0.3 x 0.3, strength 20 (0x82002C68) x the car's siren state x how squarely
    /// the lamp faces the camera. Its slot is -2 (this frame only), and for that slot AddGlow (F-92524) writes the
    /// seed (F-53016) and no star quads (F-79030): the glow has no star.
    pub fn police(&self, w: usize, pos: Vec3, facing: Vec3, color: [f32; 3], state: f32, cam: Vec3) -> Option<Glow> {
        let i = 20.0 * state * (cam - pos).normalize_or_zero().dot(facing).clamp(0.0, 1.0);
        let mut g = self.glow(w, pos, color, [0.3, 0.3], i, false, cam, 0.0)?;
        g.col_s[3] = 0.0;
        (g.pos_a[3] > 0.0).then_some(g)
    }

    /// A signal head's lit lamp: `pos` its locator, `facing` the head's z axis (F-39998:
    /// the glow sits 0.3 m behind the locator and its strength goes by how squarely the lamp
    /// faces the camera).
    pub fn signal(&self, w: usize, pos: Vec3, facing: Vec3, color: [f32; 3], cam: Vec3, tan_half_fov: f32) -> Option<Glow> {
        let (p, i) = signal_lamp(pos, facing, cam, self.signal_intensity[w])?;
        self.glow(w, p, color, [self.signal_size[w]; 2], i, true, cam, tan_half_fov)
    }

    /// The same lamp's reflection call (F-39998 with refl = 1, siglrefl.rs): the place P (0.3 m
    /// behind the locator), the strength I (as `signal`'s) and rsize = sm_fSignalGlowReflectionSizes x 0.01.
    pub fn signal_reflection(&self, w: usize, pos: Vec3, facing: Vec3, cam: Vec3) -> Option<(Vec3, f32, [f32; 2])> {
        let (p, i) = signal_lamp(pos, facing, cam, self.signal_intensity[w])?;
        Some((p, i, [self.signal_refl[w][0] * 0.01, self.signal_refl[w][1] * 0.01]))
    }

    /// A car lamp's reflection call (F-96521 with refl = 1, its caller F-27833 having set the world matrix to
    /// the mirror about the car's origin height `origin_y`): the place 1 m under the lamp, mirrored, so the
    /// rectangle rule's own mirror (siglrefl::rect) takes `origin_y`; the strength how squarely the lamp faces
    /// the camera x `k` (its level x the kind's factor - not x sm_fCarGlowIntensitys) x
    /// sm_fCarGlowReflectionIntensity; rsize = sm_CarLightGlowReflectionSizess / the picture's width and height.
    /// None under 0.001, as the main glow. (place, strength, rsize)
    pub fn car_reflection(&self, w: usize, pos: Vec3, facing: Vec3, k: f32, cam: Vec3) -> Option<(Vec3, f32, [f32; 2])> {
        let i = k * (cam - pos).normalize_or_zero().dot(facing).clamp(0.0, 1.0);
        if i <= 0.001 {
            return None;
        }
        let rs = [self.car_refl_size[w][0] / crate::siglrefl::PICTURE.0, self.car_refl_size[w][1] / crate::siglrefl::PICTURE.1];
        Some((pos - Vec3::Y, i * self.car_refl_intensity, rs))
    }
}

/// A signal head lamp's locator in the world as F-39998 makes it: the head's rows x, y, z and position summed as
/// the vector unit's four-lane dot does, (1 pos + l.z z) + (l.y y + l.x x), per component.
pub fn locator_place(rows: &[Vec3; 4], l: Vec3) -> Vec3 {
    let c = |j: usize| (rows[3][j] + l.z * rows[2][j]) + (l.y * rows[1][j] + l.x * rows[0][j]);
    Vec3::new(c(0), c(1), c(2))
}

/// F-39998's glow place and strength: P = the locator - 0.3 x the head's z axis; the strength how squarely the lamp
/// faces the camera, the unit vector to the camera as the vector unit's estimate and Newton step (0 on the place
/// itself), the dot summed x (y (z)) with fused adds, held in 0 .. 1 by two `fsel` (NaN gives 0), times `k`
/// (sm_fSignalGlowIntensitys). None unless more than 0.001. Checked on 4,000 cases against the original's code run
/// offline (`tools/signal_lamp_reference.py`, test `signal_lamp_matches_the_original`).
pub fn signal_lamp(pos: Vec3, facing: Vec3, cam: Vec3, k: f32) -> Option<(Vec3, f32)> {
    use crate::physics::exact;
    let p = pos - facing * 0.3;
    let d = exact::dot_xyz(exact::unit_or_zero(cam - p), facing);
    let a = if d >= 0.0 { d } else { 0.0 };
    let i = if a - 1.0 >= 0.0 { 1.0 } else { a } * k;
    (i > 0.001).then_some((p, i))
}

struct Halo {
    tune: Tune,
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    star: wgpu::TextureView,
    buf: wgpu::Buffer,
    count: u32,
}

static STATE: Mutex<Option<Halo>> = Mutex::new(None);

/// The rain's four glow lights (F-92524): every AddGlow call offers its place (pulled towards the camera) with the
/// key |camera - light|^2 (the light's own place, before the pull; y^2, then z and x fused on as the original sums
/// them - the projection F-75199 made beside it writes elsewhere); a key below a
/// slot's goes in there, the slots after it moving one on (the list stays sorted, four kept). Each slot: the
/// place, w = the call's strength x RainGlowIntensity, the colour. The list starts zeroed (F-96527, at start-up:
/// keys 0, so nothing goes in before the rain has read it once); the rain (F-58928) reads it and sets every
/// key back to 1e6 and every w to 0.
struct RainList {
    key: [f32; 4],
    pos: [[f32; 4]; 4],
    col: [[f32; 4]; 4],
}
static RAIN: Mutex<RainList> = Mutex::new(RainList { key: [0.0; 4], pos: [[0.0; 4]; 4], col: [[0.0; 4]; 4] });

fn rain_add(v: Vec3, p: Vec3, w: f32, color: [f32; 3]) {
    use crate::sndobj::fmadds;
    let mut l = RAIN.lock().unwrap();
    let key = fmadds(v.x, v.x, fmadds(v.z, v.z, v.y * v.y));
    let Some(i) = (0..4).find(|&i| key < l.key[i]) else { return };
    for j in (i + 1..4).rev() {
        l.key[j] = l.key[j - 1];
        l.pos[j] = l.pos[j - 1];
        l.col[j] = l.col[j - 1];
    }
    l.key[i] = key;
    l.pos[i] = [p.x, p.y, p.z, w];
    l.col[i] = [color[0], color[1], color[2], 1.0];
}

/// What the rain hands its programs as PointLights / PointLightColors; the list is then reset for the next frame.
pub fn rain_take() -> ([[f32; 4]; 4], [[f32; 4]; 4]) {
    let mut l = RAIN.lock().unwrap();
    let out = (l.pos, l.col);
    for i in 0..4 {
        l.key[i] = 1e6;
        l.pos[i][3] = 0.0;
    }
    out
}

/// MCLA_DEBUG_VIEW=115: the glows of before (the signal sprites in the scene pass, no halo pass).
pub fn old_look() -> bool {
    static OLD: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *OLD.get_or_init(|| std::env::var("MCLA_DEBUG_VIEW").map_or(false, |v| v.trim() == "115"))
}

/// The console's gamma decoding of a stored byte (the star picture's fetch constant asks for it).
fn pwl(e: u8) -> f32 {
    let e = e as f32;
    let (k, off) = if e >= 192.0 { (8.0, -1024.0) } else if e >= 96.0 { (4.0, -256.0) } else if e >= 64.0 { (2.0, -64.0) } else { (1.0, 0.0) };
    let l = e * k + off;
    (l + (l * k / 1024.0).trunc()) / 1023.0
}

/// textures/effects/star_glow.dds decoded, with its mip chain (a 2 x 2 mean a level).
fn star_texture(gpu: &Gpu, data_dir: &str) -> wgpu::TextureView {
    let p = std::path::Path::new(data_dir).join("textures/effects/star_glow.dds");
    let (mut w, mut h, mut px): (u32, u32, Vec<f32>) = match std::fs::read(&p).ok().and_then(|d| mcla_formats::dds::parse_dds("star_glow.dds", &d).ok()) {
        Some(t) if t.width > 0 && t.height > 0 && !t.levels.is_empty() => (t.width, t.height, t.to_rgba8().iter().enumerate().map(|(i, &b)| if i % 4 == 3 { b as f32 / 255.0 } else { pwl(b) }).collect()),
        _ => {
            log::warn!("star_glow.dds missing: the lamps' glows have no star");
            (1, 1, vec![0.0, 0.0, 0.0, 1.0])
        }
    };
    let mut levels: Vec<(u32, u32, Vec<u8>)> = Vec::new();
    loop {
        levels.push((w, h, px.iter().map(|v| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8).collect()));
        if w <= 1 || h <= 1 || w % 2 != 0 || h % 2 != 0 {
            break;
        }
        let (nw, nh) = (w / 2, h / 2);
        let mut next = vec![0.0f32; (nw * nh * 4) as usize];
        for y in 0..nh {
            for x in 0..nw {
                for c in 0..4 {
                    let at = |xx: u32, yy: u32| px[((yy * w + xx) * 4 + c) as usize];
                    next[((y * nw + x) * 4 + c) as usize] = 0.25 * (at(2 * x, 2 * y) + at(2 * x + 1, 2 * y) + at(2 * x, 2 * y + 1) + at(2 * x + 1, 2 * y + 1));
                }
            }
        }
        (w, h, px) = (nw, nh, next);
    }
    let (w0, h0) = (levels[0].0, levels[0].1);
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("star glow"),
        size: wgpu::Extent3d { width: w0, height: h0, depth_or_array_layers: 1 },
        mip_level_count: levels.len() as u32,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    for (i, (lw, lh, data)) in levels.iter().enumerate() {
        gpu.queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &texture, mip_level: i as u32, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            data,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(lw * 4), rows_per_image: Some(*lh) },
            wgpu::Extent3d { width: *lw, height: *lh, depth_or_array_layers: 1 },
        );
    }
    texture.create_view(&Default::default())
}

/// Once, with the city's data.
pub fn load(gpu: &mut Gpu, data_dir: &str) {
    let tune = std::fs::read_to_string(std::path::Path::new(data_dir).join("tune/lighting/GlowTuning.xml")).map(|t| Tune::parse(&t)).unwrap_or_default();
    let star = star_texture(gpu, data_dir);
    let device = &gpu.device;
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("halo layout"),
        entries: &[
            wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
            wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
            wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
        ],
    });
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("halo sampler"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });
    // (the frame's globals: the particles' pipeline has their layout)
    let globals_layout = gpu.pipeline_particle.get_bind_group_layout(0);
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("halo"), source: wgpu::ShaderSource::Wgsl(include_str!("render/halo.wgsl").into()) });
    let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("halo pl"), bind_group_layouts: &[&globals_layout, &layout], push_constant_ranges: &[] });
    const ATTRS: [wgpu::VertexAttribute; 3] = wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4, 2 => Float32x4];
    let add = wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add };
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("halo"),
        layout: Some(&pl),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_halo"),
            buffers: &[wgpu::VertexBufferLayout { array_stride: std::mem::size_of::<Glow>() as u64, step_mode: wgpu::VertexStepMode::Instance, attributes: &ATTRS }],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_halo"),
            // (blend one / one, as the original's 0x01010101)
            targets: &[Some(wgpu::ColorTargetState { format: crate::render::post::SCENE_FORMAT, blend: Some(wgpu::BlendState { color: add, alpha: add }), write_mask: wgpu::ColorWrites::ALL })],
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
        depth_stencil: None,
        multisample: Default::default(),
        multiview: None,
        cache: None,
    });
    let buf = device.create_buffer(&wgpu::BufferDescriptor { label: Some("halo glows"), size: (MAX * std::mem::size_of::<Glow>()) as u64, usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
    *STATE.lock().unwrap() = Some(Halo { tune, pipeline, layout, sampler, star, buf, count: 0 });
    crate::trail::load(gpu);
}

/// The tuning (None until `load`).
pub fn tune() -> Option<Tune> {
    STATE.lock().unwrap().as_ref().map(|h| h.tune.clone())
}

/// This frame's lights, nearest first (at most `MAX` are kept, as the game's list).
pub fn set(gpu: &Gpu, glows: &[Glow]) {
    if let Some(h) = STATE.lock().unwrap().as_mut() {
        let n = glows.len().min(MAX);
        if n > 0 {
            gpu.queue.write_buffer(&h.buf, 0, bytemuck::cast_slice(&glows[..n]));
        }
        h.count = n as u32;
    }
}

/// The halo picture cleared, nothing drawn (a car picture's frames: carpic.rs).
pub fn clear(gpu: &Gpu, enc: &mut wgpu::CommandEncoder) {
    let _ = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("halo (cleared)"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: gpu.post.halo_view(), resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store } })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
    });
}

/// The pass: after the scene (its depth is read), before the tone map (which adds the
/// picture). Always clears the halo picture.
pub fn run(gpu: &Gpu, enc: &mut wgpu::CommandEncoder) {
    let guard = STATE.lock().unwrap();
    let bind = guard.as_ref().filter(|h| h.count > 0).map(|h| {
        gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("halo"),
            layout: &h.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&h.star) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&h.sampler) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&gpu.depth) },
            ],
        })
    });
    let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("halo"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: gpu.post.halo_view(), resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store } })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
    });
    if let (Some(h), Some(bind)) = (guard.as_ref(), bind.as_ref()) {
        pass.set_pipeline(&h.pipeline);
        pass.set_bind_group(0, &gpu.globals_bg, &[]);
        pass.set_bind_group(1, bind, &[]);
        pass.set_vertex_buffer(0, h.buf.slice(..));
        pass.draw(0..24, 0..h.count);
    }
    // (the light trails: drawn by the original after the glows, into the same picture)
    crate::trail::draw(gpu, &mut pass);
}

#[cfg(test)]
mod signal_lamp_tests {
    use super::*;

    #[test]
    fn signal_lamp_matches_the_original() {
        let t = include_str!("testdata/signal_lamp_reference.txt");
        let h = |x: &str| u32::from_str_radix(x, 16).unwrap();
        let f = |x: &str| f32::from_bits(h(x));
        let lines: Vec<&str> = t.lines().collect();
        let (mut n, mut calls) = (0, 0);
        for (k, l) in lines.iter().enumerate() {
            let Some(c) = l.strip_prefix("CASE ") else { continue };
            let w: Vec<&str> = c.split_whitespace().collect();
            let v = |i: usize| Vec3::new(f(w[i]), f(w[i + 1]), f(w[i + 2]));
            let rows = [v(0), v(4), v(8), v(12)];
            let t = locator_place(&rows, v(16));
            let got = signal_lamp(t, rows[2], v(20), f(w[23]));
            let next = lines.get(k + 1).filter(|x| !x.starts_with("CASE"));
            match (got, next) {
                (None, None) => {}
                (Some((p, i)), Some(x)) => {
                    let e: Vec<&str> = x.split_whitespace().collect();
                    let want = [h(e[1]), h(e[2]), h(e[3]), h(e[4])];
                    assert_eq!([p.x.to_bits(), p.y.to_bits(), p.z.to_bits(), i.to_bits()], want, "{l}");
                    if h(w[19]) == 1 {
                        assert_eq!(e[0], "REFL", "{l}");
                        assert_eq!([(f(w[25]) * 0.01).to_bits(), (f(w[26]) * 0.01).to_bits()], [h(e[5]), h(e[6])], "{l}");
                    } else {
                        assert_eq!(e[0], "GLOW", "{l}");
                        assert_eq!(([h(e[5]), h(e[6]), h(e[7])], e[8]), ([0x3f00_0000, h(w[24]), h(w[24])], "-3"), "{l}");
                    }
                    calls += 1;
                }
                (g, x) => panic!("{l}: port {g:?}, original {x:?}"),
            }
            n += 1;
        }
        assert_eq!((n, calls), (4000, 1604));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_list_visit_matches_the_original() {
        // tools/lamp_list_reference.py: F-79256 run whole on 3,000 lists; every call it makes (the view's sphere test,
        // the occlusion with its sphere and mode, the callback with the manager's flags, value and distance) in order
        let text = include_str!("testdata/lamp_list_reference.txt");
        let hx = |s: &str| u32::from_str_radix(s, 16).unwrap();
        let fl = |s: &str| f32::from_bits(hx(s));
        let mut blocks: Vec<Vec<&str>> = Vec::new();
        for l in text.lines() {
            if l.starts_with("CASE") {
                blocks.push(Vec::new());
            }
            blocks.last_mut().unwrap().push(l);
        }
        let (mut n, mut listed, mut fails) = (0, 0, 0);
        for b in &blocks {
            let w: Vec<&str> = b[0].split_whitespace().collect();
            let cam = Vec3::new(fl(w[3]), fl(w[4]), fl(w[5]));
            let k = [fl(w[7]), fl(w[8]), fl(w[9])];
            let far = fl(w[11]);
            let mut mine: Vec<String> = Vec::new();
            for l in b.iter().filter(|l| l.starts_with("N ")) {
                let d: Vec<&str> = l.split_whitespace().collect();
                let i: usize = d[1].parse().unwrap();
                let hw: u32 = d[2].parse().unwrap();
                if hw & 0x8000 == 0 {
                    continue;
                }
                let (kind, skip, w156): (u32, u32, u32) = (d[3].parse().unwrap(), d[4].parse().unwrap(), hx(d[5]));
                let sph = (Vec3::new(fl(d[7]), fl(d[8]), fl(d[9])), fl(d[10]));
                let (vis, occ): (u32, u32) = (d[11].parse().unwrap(), hx(d[12]));
                let calls = std::cell::RefCell::new(Vec::new());
                let v = visit(cam, k, far, kind as i32, skip as u8, w156 != 0, fl(d[6]), sph,
                    || { calls.borrow_mut().push(format!("VIS {i} 00001400")); vis != 0 },
                    || { calls.borrow_mut().push(format!("OCC {i} {} {} {} {} 2", d[7], d[8], d[9], d[10])); occ });
                mine.extend(calls.into_inner());
                if let Some(v) = v {
                    mine.push(format!("CB {i} {:08x} {:08x} {:08x} 00000000", v.flags, v.value.to_bits(), v.dist.to_bits()));
                    listed += 1;
                }
            }
            let theirs: Vec<String> = b.iter().filter(|l| !l.starts_with("N ") && !l.starts_with("CASE")).map(|l| l.to_string()).collect();
            if mine != theirs {
                if fails < 4 {
                    eprintln!("{}: port {:?} original {:?}", b[0], mine, theirs);
                }
                fails += 1;
            }
            n += 1;
        }
        assert!(n == 3000 && listed > 2000 && fails == 0, "{fails} of {n} lists differ ({listed} listed)");
    }

    #[test]
    fn the_gate_lamp_as_recorded() {
        // capture hl_L_23.5: the lamp at 817.46 26.20 -805.12 seen from 813.65 19.57 -814.71 has its
        // quads at 817.14 25.65 -805.92, alpha bytes 110 (seed) and 16 (star), seed 27 x 13.8,
        // star half diagonals 80.19 and 23.91 (x 0.05 in the vertex program)
        let t = Tune::default();
        let g = t.prop(0, Vec3::new(817.46, 26.20, -805.12), Vec3::new(1.0, 0.937, 0.5), Vec3::new(813.65, 19.57, -814.71)).unwrap();
        assert!((g.pos_a[0] - 817.14).abs() < 0.02 && (g.pos_a[1] - 25.65).abs() < 0.02 && (g.pos_a[2] + 805.92).abs() < 0.02, "{:?}", g.pos_a);
        assert_eq!((g.pos_a[3] * 255.0).round() as i32, 110);
        assert_eq!((g.col_s[3] * 255.0).round() as i32, 16);
        assert!((g.size[0] - 2.7).abs() < 1e-4 && (g.size[1] - 1.38).abs() < 1e-4);
        assert!((g.size[2] / 0.05 - 80.19).abs() < 0.01 && (g.size[3] / 0.05 - 23.91).abs() < 0.01, "{:?}", g.size);
        // foggy: seed 102, star 7, size 23.19 x 14.9
        let g = t.prop(3, Vec3::new(817.46, 26.20, -805.12), Vec3::ONE, Vec3::new(813.65, 19.57, -814.71)).unwrap();
        assert_eq!(((g.pos_a[3] * 255.0).round() as i32, (g.col_s[3] * 255.0).round() as i32), (102, 7));
    }

    #[test]
    fn a_signal_grows_with_distance() {
        // the same capture: red lamps 100.5 m off are 6.37 across, 129.7 m off 7.51 (4.52 at the lamp)
        let t = Tune::default();
        let tan = (25.0f32.to_radians() * 0.5).tan();
        for (d, s) in [(100.5f32, 6.37f32), (129.66, 7.51)] {
            let g = t.signal(0, Vec3::new(0.0, 0.0, -d + 0.3), Vec3::Z, [1.0, 0.0, 0.0], Vec3::ZERO, tan).unwrap();
            assert!((g.size[0] * 10.0 - s).abs() < 0.03, "{d}: {}", g.size[0] * 10.0);
            assert_eq!((g.pos_a[3] * 255.0).round() as i32, 115);
        }
        // seen from behind: none
        assert!(t.signal(0, Vec3::new(0.0, 0.0, 50.0), Vec3::Z, [1.0, 0.0, 0.0], Vec3::ZERO, tan).is_none());
    }

    /// F-59032's packing: clamp 0 .. 1, x 255, cut; the R8's tail lamp (3.729, 0.119, 0) is the
    /// rf21w recording's rain light (1, 0.1176, 0); a packed colour passes unchanged.
    #[test]
    fn glow_colour_is_packed_to_bytes() {
        assert_eq!(pack([3.729, 0.119, 0.0]), [1.0, 30.0 / 255.0, 0.0]);
        assert_eq!(pack([-0.5, 0.999, 0.5]), [0.0, 254.0 / 255.0, 127.0 / 255.0]);
        for k in 0..=255u8 {
            let v = k as f32 / 255.0;
            assert_eq!(pack([v; 3]), [v; 3]);
        }
    }

    #[test]
    fn the_tuning_file_is_read() {
        let x = "<GlowTuning><sm_fPropGlowIntensitys content=\"float_array\">\n1.5\n2\n3\n4\n</sm_fPropGlowIntensitys><sm_fBloomIntensitys><Item content=\"float_array\">\n1\n1\n1\n3\n</Item></sm_fBloomIntensitys><StarSize content=\"vector2_array\">\n6 3\n6 3\n6 3\n6 2\n</StarSize><MaxGlowBias value=\"0.07\"/></GlowTuning>";
        let t = Tune::parse(x);
        assert_eq!(t.prop_intensity, [1.5, 2.0, 3.0, 4.0]);
        assert_eq!(t.bloom[3], 3.0);
        assert_eq!(t.bloom_items[1], [1.0, 1.0, 1.0, 3.0]);
        assert_eq!(t.star[3], [6.0, 2.0]);
        assert_eq!(t.max_bias, 0.07);
        assert_eq!(t.min_bias, 0.009);
    }

    /// The disc's file: both items of the nested arrays, and the weather sum.
    #[test]
    fn glow_tune_items_and_weather_sum() {
        let root = std::env::var("MCLA_DATA_DIR").unwrap_or_else(|_| "D:/MCLA/extracted/cache".into());
        let Ok(x) = std::fs::read_to_string(std::path::Path::new(&root).join("tune/lighting/GlowTuning.xml")) else { return };
        let mut t = Tune::parse(&x);
        assert_eq!(t.streak_items, [[0.15; 4]; 2]);
        assert_eq!(t.bloom_items, [[1.0, 1.0, 1.0, 2.0]; 2]);
        // nice -> foggy, half way: the prop glow's intensity, size and bloom between the two
        t.night_item(true);
        t.blend(0, 3, 0.5);
        assert!((t.prop_intensity[0] - (4.35 + 2.0) / 2.0).abs() < 1e-6);
        assert!((t.prop_size[0][1] - (13.8 + 14.9) / 2.0).abs() < 1e-5);
        assert!((t.bloom[0] - 1.5).abs() < 1e-6);
        assert!((t.star[0][1] - (3.85 + 3.2) / 2.0).abs() < 1e-6);
        assert_eq!(t.prop_intensity[3], 2.0);
        assert_eq!(t.prop_refl_size, [100.0, 100.0]);
        assert_eq!(t.prop_refl_intensity, 3.8);
        assert_eq!(t.car_refl_size[2], [20.0, 30.0]);
        assert_eq!(t.car_refl_size[0], [0.0, 0.0]);
        assert_eq!(t.car_refl_intensity, 1.55);
        assert_eq!(t.night_light, 1.5);
    }

    /// A car lamp's reflection call (F-96521, refl = 1): 1 m under the lamp, strength = facing x level x 1.55, the
    /// size the weather's over 320 x 180 - zero but in Stormy, so the rectangle is the draw's one pixel (the
    /// original's eight car rectangles of hl_L_23.5: 0.00312 x 0.00556 NDC = 1/320 x 1/180).
    #[test]
    fn car_reflection_call() {
        let t = Tune::default();
        let cam = Vec3::new(0.0, 1.0, -10.0);
        let (p, i, rs) = t.car_reflection(0, Vec3::new(0.0, 1.0, 0.0), Vec3::NEG_Z, 0.5, cam).unwrap();
        assert_eq!(p, Vec3::ZERO);
        assert!((i - 0.5 * 1.55).abs() < 1e-6);
        assert_eq!(rs, [0.0, 0.0]);
        let (_, _, rs) = t.car_reflection(2, Vec3::new(0.0, 1.0, 0.0), Vec3::NEG_Z, 0.5, cam).unwrap();
        assert!((rs[0] - 20.0 / 320.0).abs() < 1e-7 && (rs[1] - 30.0 / 180.0).abs() < 1e-7);
        // (facing away: nothing)
        assert!(t.car_reflection(0, Vec3::new(0.0, 1.0, 0.0), Vec3::Z, 0.5, cam).is_none());
        let r = crate::siglrefl::rect(glam::Mat4::perspective_infinite_reverse_rh(1.0, 16.0 / 9.0, 0.2), 0.2, 0.0, Vec3::new(0.0, -2.0, -50.0), Vec3::ZERO, [1.0; 3], 1.0, rs.map(|_| 0.0), crate::siglrefl::PICTURE).unwrap();
        assert!((r.r[2] - r.r[0] - 1.0 / 320.0).abs() < 1e-6 && (r.r[3] - r.r[1] - 1.0 / 180.0).abs() < 1e-6);
    }
}
