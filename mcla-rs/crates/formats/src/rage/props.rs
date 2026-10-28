//! City props.
//!
//! * `resources/prop/0x0e66973c.r131` – dictionary (root+0x08) of 230 prop fragment
//!   types keyed by joaat(name). A fragment type has its name at +0xb0 and its
//!   drawable at +0xb4; the drawable holds the shader group at +0x08, its bounding
//!   sphere (centre +0x10, radius +0x70) and box (+0x20 / +0x30) and, at +0x40, four
//!   pointers to LOD slots `{-> model ptrs, u16 count, u16 cap}` (all but the two hedge
//!   types fill only the first; 21 types - collision posts, particle props, box sets -
//!   have no model at all).
//! * `resources/prop/0x982ff5e0.r134` – foliage placement: one 224-byte entry per tree /
//!   bush type with its instance buffer (see [`parse_foliage`]; MAP_WORK.md has how the
//!   layout was checked against the running original).

use super::drawable::Shader;
use super::geometry::{decode, Mesh};
use super::scan::geometry_at;
use super::{Dictionary, Rd};
use crate::rsc::Resource;
use glam::Vec3;

pub struct Prop {
    /// offset of the fragment type object (instances refer to it)
    pub off: usize,
    pub name: String,
    pub hash: u32,
    /// LOD0 meshes with their shader index
    pub meshes: Vec<(Mesh, u16)>,
    /// the next LOD slot's meshes (only the two hedge types have one: their low level)
    pub lod1: Vec<(Mesh, u16)>,
    pub shaders: Vec<Shader>,
    pub min: Vec3,
    pub max: Vec3,
    /// the drawable's bounding sphere (zero for the types without a model)
    pub center: Vec3,
    pub radius: f32,
    /// mass of the whole prop (kg)
    pub mass: f32,
    /// the parts of its collision shape
    pub parts: Vec<BoundPart>,
    /// named points of the type, in its frame (`light0` on lamps; `red`, `red1`, `yellow`,
    /// `yellow1`, `green`, `green1`, `walk`, `walk1` on signal heads; `break_ll` .. on panes)
    pub locators: Vec<(String, Vec3)>,
}

/// One part of a prop's composite collision bound, as a box in the prop's frame.
#[derive(Debug, Clone, Copy)]
pub struct BoundPart {
    /// 0 sphere, 1 capsule, 3 box, 4 polyhedron
    pub kind: u8,
    pub min: Vec3,
    pub max: Vec3,
    /// mass of the fragment child it belongs to
    pub mass: f32,
}

impl Prop {
    /// Things that stand fast - lamp posts, masts, trees, the unseen posts at building columns -
    /// are two upright capsules: a wide one of 100 kg (2.5 m radius on most: the zone in which
    /// the original eases a car past the post) around the post itself, 900 kg. Returns the post.
    pub fn post(&self) -> Option<&BoundPart> {
        (self.parts.len() == 2 && self.parts.iter().all(|p| p.kind == 1)).then(|| &self.parts[1])
    }
}

/// Fragment type: +e4 -> archetype (class 0x6a951c, +28 mass); +ec -> composite bound (class
/// 0x6a09dc: +80 -> part pointers, +84 -> 64-byte part matrices, +90 u16 count; a part: +04 type,
/// +10 box max, +20 box min); +d4 -> child pointers (child +04 mass), their number at +12f.
fn physics(rd: &Rd, frag: usize) -> (f32, Vec<BoundPart>) {
    let mass = rd.ptr(frag + 0xe4).filter(|a| rd.vtable(*a) == 0x006a_951c).map_or(0.0, |a| rd.f32(a + 0x28));
    let mut parts = Vec::new();
    let Some(b) = rd.ptr(frag + 0xec).filter(|b| rd.vtable(*b) == 0x006a_09dc) else { return (mass, parts) };
    let (Some(arr), n) = (rd.ptr(b + 0x80), rd.u16(b + 0x90) as usize) else { return (mass, parts) };
    let mats = rd.ptr(b + 0x84);
    let (children, n_children) = (rd.ptr(frag + 0xd4), rd.u8(frag + 0x12f) as usize);
    let v = |o: usize| Vec3::new(rd.f32(o), rd.f32(o + 4), rd.f32(o + 8));
    for k in 0..n.min(64) {
        let Some(p) = rd.ptr(arr + k * 4) else { continue };
        let (mx, mn) = (v(p + 0x10), v(p + 0x20));
        let m = match mats {
            Some(m) if rd.has(m + k * 64, 64) => glam::Affine3A::from_cols(v(m + k * 64).into(), v(m + k * 64 + 16).into(), v(m + k * 64 + 32).into(), v(m + k * 64 + 48).into()),
            _ => glam::Affine3A::IDENTITY,
        };
        if !(mn.is_finite() && mx.is_finite() && m.is_finite()) {
            continue;
        }
        let (mut lo, mut hi) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
        for c in 0..8 {
            let q = m.transform_point3(Vec3::new(if c & 1 == 0 { mn.x } else { mx.x }, if c & 2 == 0 { mn.y } else { mx.y }, if c & 4 == 0 { mn.z } else { mx.z }));
            lo = lo.min(q);
            hi = hi.max(q);
        }
        let child_mass = children.filter(|_| k < n_children).and_then(|c| rd.ptr(c + k * 4)).map_or(0.0, |c| rd.f32(c + 4));
        parts.push(BoundPart { kind: rd.u8(p + 4), min: lo, max: hi, mass: child_mass });
    }
    (mass, parts)
}

/// The locators of a fragment type: a search tree of 64-byte nodes sorted by name, its root
/// at drawable +d4 and their number at +d8. Node: +00 position, +10 rotation (three angles),
/// +30 -> name, +34 -> parent, +38 -> left, +3c -> right. 59 of the 230 types have one.
fn locators(rd: &Rd, drawable: usize) -> Vec<(String, Vec3)> {
    let mut out = Vec::new();
    let n = (rd.u32(drawable + 0xd8) as usize).min(64);
    let mut stack: Vec<usize> = rd.ptr(drawable + 0xd4).filter(|_| n > 0).into_iter().collect();
    while let Some(o) = stack.pop() {
        if out.len() >= n || !rd.has(o, 64) {
            break;
        }
        let p = Vec3::new(rd.f32(o), rd.f32(o + 4), rd.f32(o + 8));
        if let (Some(name), true) = (rd.cstr_ptr(o + 0x30), p.is_finite()) {
            out.push((name, p));
        }
        stack.extend([rd.ptr(o + 0x38), rd.ptr(o + 0x3c)].into_iter().flatten());
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

fn model_meshes(rd: &Rd, model: usize, out: &mut Vec<(Mesh, u16)>) {
    let cnt = rd.u16(model + 0x08) as usize;
    let Some(arr) = rd.ptr(model + 0x04) else { return };
    let si = rd.ptr(model + 0x10);
    for i in 0..cnt.min(256) {
        let Some(g) = rd.ptr(arr + i * 4) else { continue };
        let Some(gr) = geometry_at(rd, g) else { continue };
        if let Ok(mesh) = decode(rd, &gr.vb, &gr.ib) {
            let s = si.map(|p| rd.u16(p + i * 2)).unwrap_or(0);
            out.push((mesh, s));
        }
    }
}

/// Parse every prop of the prop dictionary resource.
pub fn parse_props(res: &Resource) -> Vec<Prop> {
    let rd = Rd::new(res);
    let mut out = Vec::new();
    let Some(d) = rd.ptr(0x08) else { return out };
    let Ok(dict) = Dictionary::parse(&rd, d) else { return out };
    for (h, v) in dict.hashes.iter().zip(dict.values.iter()) {
        let Some(frag) = *v else { continue };
        let name = rd.cstr_ptr(frag + 0xb0).unwrap_or_default();
        let Some(dr) = rd.ptr(frag + 0xb4) else { continue };
        let mut shaders = Vec::new();
        if let Some(sg) = rd.ptr(dr + 0x08) {
            let count = rd.u16(sg + 0x0c) as usize;
            if let Some(arr) = rd.ptr(sg + 0x08) {
                for i in 0..count.min(64) {
                    match rd.ptr(arr + i * 4).and_then(|so| Shader::parse(&rd, so).ok()) {
                        Some(s) => shaders.push(s),
                        None => shaders.push(Shader { name: String::new(), name_hash: 0, source_path: String::new(), params: vec![], bucket: 0 }),
                    }
                }
            }
        }
        let mut meshes = Vec::new();
        let mut lod1 = Vec::new();
        {
            // first LOD slot that has models (and the slot after it)
            for lod in 0..4 {
                let Some(slot) = rd.ptr(dr + 0x40 + lod * 4) else { continue };
                let n = rd.u16(slot + 4) as usize;
                let Some(arr) = rd.ptr(slot) else { continue };
                if n == 0 || n > 64 {
                    continue;
                }
                for i in 0..n {
                    if let Some(m) = rd.ptr(arr + i * 4) {
                        model_meshes(&rd, m, &mut meshes);
                    }
                }
                if !meshes.is_empty() {
                    if let Some(slot) = rd.ptr(dr + 0x40 + (lod + 1) * 4).filter(|_| lod < 3) {
                        let n = rd.u16(slot + 4) as usize;
                        if let (Some(arr), true) = (rd.ptr(slot), n > 0 && n <= 64) {
                            for i in 0..n {
                                if let Some(m) = rd.ptr(arr + i * 4) {
                                    model_meshes(&rd, m, &mut lod1);
                                }
                            }
                        }
                    }
                    break;
                }
            }
        }
        let mut min = Vec3::splat(f32::INFINITY);
        let mut max = Vec3::splat(f32::NEG_INFINITY);
        for (m, _) in &meshes {
            let (a, b) = m.bbox();
            min = min.min(a);
            max = max.max(b);
        }
        let center = Vec3::new(rd.f32(dr + 0x10), rd.f32(dr + 0x14), rd.f32(dr + 0x18));
        let radius = rd.f32(dr + 0x70);
        let (center, radius) = if center.is_finite() && radius.is_finite() && radius > 0.0 { (center, radius) } else { (Vec3::ZERO, 0.0) };
        let (mass, parts) = physics(&rd, frag);
        let locators = locators(&rd, dr);
        out.push(Prop { off: frag, name, hash: *h, meshes, lod1, shaders, min, max, center, radius, mass, parts, locators });
    }
    out
}

/// One plant as the original draws it.
#[derive(Debug, Clone, Copy)]
pub struct FoliageInstance {
    pub pos: Vec3,
    /// the model's y axis in the world: its direction is the plant's lean, its length the
    /// height scale
    pub up: Vec3,
    /// scale of the model's x and z axes
    pub width: f32,
    /// rotation step about the plant's own axis (hedges only; 0 on every other type)
    pub rot: u8,
    /// colour of the night fill light at the plant (bytes +0d..+0f): the night ambient map's
    /// value at its place, baked in (`aomap::AoMap::color_at`; the original hands it to the
    /// plant's programs as gPropNFLColor.rgb / in the cards' vertex stream)
    pub fill: [u8; 3],
    /// the plant's own lamp at night (bytes +14..+1f)
    pub lamp: FoliageLamp,
}

/// The one lamp the original's night programs light a plant with, baked into its record
/// (LIGHTING_WORK.md, "The plants' own lamp at night"). The far cards' program
/// (VSPropInstanceFoliageNight) reads it from the instance stream, for a plant drawn as a
/// model the game copies it into c46 / c45 / c41 - checked on 55 plants of five types, exact:
///
/// ```text
/// +14 u8 cone, inner   +15..17 colour r g b (/ 255)
/// +18 u8 strength      +19..1b position x y z, relative to the plant:
///                      byte / 255 * type +40 [c] + type +50 [c]  (gMPropLightPositionScale / Bias; w = strength)
/// +1c u8 cone, outer   +1d..1f direction x y z (byte / 255 * 2 - 1): from the lit point towards the lamp
/// light = colour x sat(n.l) x sat(strength / d^2) x sat((l . direction - o) / (i - o)),
///         i, o = the cone bytes / 255 * 2 - 1; both <= 0 (127, 127 on 83 % of the plants): no cone
/// ```
/// Strength 0 (16 % of the plants): no lamp.
#[derive(Debug, Clone, Copy, Default)]
pub struct FoliageLamp {
    /// position relative to the plant's
    pub pos: Vec3,
    pub strength: f32,
    pub color: [u8; 3],
    pub dir: [u8; 3],
    /// (inner, outer) cone bytes
    pub cone: [u8; 2],
}

impl FoliageInstance {
    /// Height scale.
    pub fn height(&self) -> f32 {
        self.up.length()
    }

    /// The plant's unit axes (x, y, z columns): y along `up`, x and z level-ish and square to
    /// it - x = up x world z, as in the world matrices the original uploads - then turned
    /// about y by `rot / steps` of a circle.
    pub fn rotation(&self, steps: u32) -> glam::Quat {
        let y = self.up.normalize_or(Vec3::Y);
        let x = y.cross(Vec3::Z).normalize_or(Vec3::X);
        let z = x.cross(y);
        let q = glam::Quat::from_mat3(&glam::Mat3::from_cols(x, y, z)).normalize();
        if self.rot == 0 || steps <= 1 {
            q
        } else {
            (q * glam::Quat::from_rotation_y(self.rot as f32 / steps as f32 * std::f32::consts::TAU)).normalize()
        }
    }
}

#[derive(Debug, Clone)]
pub struct FoliageType {
    /// prop name, some with the suffix `_displayAsLowLOD`
    pub name: String,
    /// rotation steps of a full turn: 250 on the hedge types, 1 (never turned) on the others
    pub rot_steps: u32,
    pub instances: Vec<FoliageInstance>,
}

/// Parse the foliage placement resource: root +08 -> array of 224-byte type entries (u16
/// count at root +0c):
///
/// ```text
/// +40 vec4 scale, +50 vec4 bias   of the instances' lamp position (xyz) and strength (w)
/// +60 vec4 scale, +70 vec4 bias   the original shader's gMPropUpAndXZScaleScale / Bias
/// +80 -> name                     +90 u32 rotation steps (MaxRots of MultiPropTuning.xml)
/// +b4 -> culling tree, 18-byte nodes with 16-bit float cluster centres (not read here;
///        earlier versions took its leaves for the plants: wrong place, size and turn)
/// +c8 -> vertex buffer {+04 u16 count, +08 -> data, +0c stride 32}: the instances
///        +00 f32 x y z   +0c u8 rotation step   +10 u8 width, +11..13 u8 up x y z
///        (value = byte / 255 * scale[c] + bias[c], c = w, x, y, z)
/// ```
pub fn parse_foliage(res: &Resource) -> Vec<FoliageType> {
    let rd = Rd::new(res);
    let mut out: Vec<FoliageType> = Vec::new();
    let Some(types) = rd.ptr(0x08) else { return out };
    let n = rd.u16(0x0c) as usize;
    for k in 0..n.min(1024) {
        let o = types + k * 224;
        if !rd.has(o, 224) {
            break;
        }
        let Some(name) = rd.cstr_ptr(o + 0x80) else { continue };
        let Some(vb) = rd.ptr(o + 0xc8) else { continue };
        let (count, stride) = (rd.u16(vb + 4) as usize, rd.u32(vb + 0x0c) as usize);
        let Some(data) = rd.ptr(vb + 8) else { continue };
        if stride < 20 || !rd.has(data, count * stride) {
            continue;
        }
        let f4 = |p: usize| [rd.f32(p), rd.f32(p + 4), rd.f32(p + 8), rd.f32(p + 12)];
        let (scale, bias) = (f4(o + 0x60), f4(o + 0x70));
        let (lscale, lbias) = (f4(o + 0x40), f4(o + 0x50));
        let mut instances = Vec::with_capacity(count);
        for i in 0..count {
            let r = data + i * stride;
            let v = |c: usize, b: usize| rd.u8(r + 0x10 + b) as f32 / 255.0 * scale[c] + bias[c];
            let it = FoliageInstance { pos: Vec3::new(rd.f32(r), rd.f32(r + 4), rd.f32(r + 8)), up: Vec3::new(v(0, 1), v(1, 2), v(2, 3)), width: v(3, 0), rot: rd.u8(r + 0x0c), fill: [rd.u8(r + 0x0d), rd.u8(r + 0x0e), rd.u8(r + 0x0f)], lamp: FoliageLamp::default() };
            let mut it = it;
            if stride >= 32 {
                let l = |c: usize| rd.u8(r + 0x18 + (c + 1) % 4) as f32 / 255.0 * lscale[c] + lbias[c];
                let lamp = FoliageLamp { pos: Vec3::new(l(0), l(1), l(2)), strength: l(3), color: [rd.u8(r + 0x15), rd.u8(r + 0x16), rd.u8(r + 0x17)], dir: [rd.u8(r + 0x1d), rd.u8(r + 0x1e), rd.u8(r + 0x1f)], cone: [rd.u8(r + 0x14), rd.u8(r + 0x1c)] };
                if lamp.pos.is_finite() && lamp.strength.is_finite() {
                    it.lamp = lamp;
                }
            }
            if it.pos.is_finite() && it.up.is_finite() && it.width.is_finite() {
                instances.push(it);
            }
        }
        out.push(FoliageType { name, rot_steps: rd.u32(o + 0x90).max(1), instances });
    }
    out
}

/// One placed prop: root+0x0c -> pointer array (u16 count at root+0x10) of instance
/// objects `{+0x10 x axis, +0x20 y axis, +0x30 z axis, +0x40 position, +0x5c -> fragment type}`.
#[derive(Debug, Clone, Copy)]
pub struct PropInstance {
    /// offset of the fragment type (see [`Prop::off`])
    pub frag: usize,
    pub pos: Vec3,
    pub rot: glam::Quat,
    /// instance +80: 1 = loose (a car knocks it away: 13,233 instances of 111 types), 2 = stands
    /// fast (8,990: every lamp, mast, tree, post, and the particle props), 3 = a glow card (373).
    /// The same for every instance of a type.
    pub kind: u32,
}

pub fn parse_prop_instances(res: &Resource) -> Vec<PropInstance> {
    let rd = Rd::new(res);
    let mut out = Vec::new();
    let Some(arr) = rd.ptr(0x0c) else { return out };
    let n = rd.u16(0x10) as usize;
    for i in 0..n {
        let Some(o) = rd.ptr(arr + i * 4) else { continue };
        let Some(frag) = rd.ptr(o + 0x5c) else { continue };
        let v = |k: usize| Vec3::new(rd.f32(o + k), rd.f32(o + k + 4), rd.f32(o + k + 8));
        let m = glam::Mat3::from_cols(v(0x10).normalize_or(Vec3::X), v(0x20).normalize_or(Vec3::Y), v(0x30).normalize_or(Vec3::Z));
        let pos = v(0x40);
        if !pos.is_finite() {
            continue;
        }
        out.push(PropInstance { frag, pos, rot: glam::Quat::from_mat3(&m).normalize(), kind: rd.u32(o + 0x80) });
    }
    out
}

#[cfg(test)]
mod foliage_tests {
    use super::*;

    fn file() -> Option<Resource> {
        let root = std::env::var("MCLA_DATA_DIR").unwrap_or_else(|_| "D:/MCLA/extracted/cache".into());
        let d = std::fs::read(std::path::Path::new(&root).join("resources/prop/0x982ff5e0.r134")).ok()?;
        if d.len() < 16 || &d[0..4] != b"MCLR" {
            return None;
        }
        let u = |o: usize| u32::from_le_bytes(d[o..o + 4].try_into().unwrap());
        let data = d[16..].to_vec();
        let vs = (u(12) as usize).min(data.len());
        Some(Resource { rtype: u(4), flags: u(8), virtual_size: vs, physical_size: data.len() - vs, data })
    }

    /// The plants the running original drew as models in the recorded frame
    /// (work/lighting/cap/p12.bin): position, width scale and up row of their world matrices.
    #[test]
    fn instances_match_the_recorded_original() {
        let Some(res) = file() else { return };
        let types = parse_foliage(&res);
        assert_eq!(types.len(), 53);
        assert_eq!(types.iter().map(|t| t.instances.len()).sum::<usize>(), 95523);
        let want: [(&str, [f32; 3], f32, [f32; 3]); 4] = [
            ("la_prop_gfx_bush_01x", [819.5843, 18.5199, -849.8146], 1.5626, [-0.0027, 1.7840, 0.0169]),
            ("la_prop_gfx_tall_bush_01x", [818.7812, 16.0717, -852.8406], 0.7954, [0.0014, 0.7931, 0.0026]),
            ("la_prop_gfx_tree_03x", [835.6204, 18.1500, -852.2572], 0.9997, [0.0004, 0.9961, 0.0003]),
            ("la_prop_gfx_tree_04x", [825.1862, 18.6334, -850.5622], 1.2420, [-0.0014, 1.2420, 0.0015]),
        ];
        for (name, pos, width, up) in want {
            let t = types.iter().find(|t| t.name == name).unwrap();
            let p = Vec3::from(pos);
            let it = t.instances.iter().min_by(|a, b| a.pos.distance(p).total_cmp(&b.pos.distance(p))).unwrap();
            assert!(it.pos.distance(p) < 2e-3, "{name}: {:?}", it.pos);
            assert!((it.width - width).abs() < 2e-3, "{name}: width {}", it.width);
            assert!(it.up.distance(Vec3::from(up)) < 3e-3, "{name}: up {:?}", it.up);
            // its x axis lies in the world's xy plane, as recorded
            let x = it.rotation(t.rot_steps) * Vec3::X;
            assert!(x.z.abs() < 1e-4 && x.x > 0.999, "{name}: x axis {:?}", x);
        }
    }

    /// The collision of three prop types as the fragment types hold it.
    #[test]
    fn prop_physics_from_the_fragment_types() {
        let root = std::env::var("MCLA_DATA_DIR").unwrap_or_else(|_| "D:/MCLA/extracted/cache".into());
        let Ok(d) = std::fs::read(std::path::Path::new(&root).join("resources/prop/0x0e66973c.r131")) else { return };
        let u = |o: usize| u32::from_le_bytes(d[o..o + 4].try_into().unwrap());
        let data = d[16..].to_vec();
        let vs = (u(12) as usize).min(data.len());
        let props = parse_props(&Resource { rtype: u(4), flags: u(8), virtual_size: vs, physical_size: data.len() - vs, data });
        assert_eq!(props.len(), 230);
        let get = |n: &str| props.iter().find(|p| p.name == n).unwrap();
        // the locators: 59 types have some; a signal head's lamps
        assert_eq!(props.iter().filter(|p| !p.locators.is_empty()).count(), 59);
        let head = get("la_prop_static_traffic_signal_light_09x");
        assert_eq!(head.locators.iter().map(|l| l.0.as_str()).collect::<Vec<_>>(), ["green", "green1", "green2", "light0", "red", "red1", "red2", "yellow", "yellow1"]);
        let at = |n: &str| head.locators.iter().find(|l| l.0 == n).unwrap().1;
        assert!(at("red").distance(Vec3::new(-9.724, 8.073, 0.577)) < 2e-3 && at("green1").distance(Vec3::new(0.087, 3.200, 0.845)) < 2e-3);
        assert_eq!(get("la_prop_static_traffic_signal_pole_01x").locators.len(), 3);
        // a street tree: the trunk is a capsule of 0.336 m
        let tree = get("la_prop_gfx_tree_03x");
        let post = tree.post().unwrap();
        assert!((tree.mass - 1000.0).abs() < 0.5 && ((post.max.x - post.min.x) * 0.5 - 0.336).abs() < 2e-3 && (post.mass - 900.0).abs() < 0.5);
        // an unseen post at a building's column: no model, 0.7 m radius, 8.4 m tall
        let col = get("la_prop_static_column_sm_01x");
        assert!(col.meshes.is_empty() && col.radius == 0.0);
        let post = col.post().unwrap();
        assert!(((post.max.x - post.min.x) * 0.5 - 0.7).abs() < 1e-3 && (post.max.y - post.min.y - 8.4).abs() < 1e-2);
        // a bin: boxes, 40 kg, knocked over
        let bin = get("la_prop_trash_can_04x");
        assert!(bin.post().is_none() && (bin.mass - 40.0).abs() < 0.1 && bin.parts.len() == 4 && bin.parts.iter().all(|p| p.kind == 3));
        // the drawable's sphere: what the fade distance is taken from
        assert!((get("la_prop_sign_custparking_01x").radius - 1.501).abs() < 2e-3);
    }

    /// Only hedges are ever turned, and their step count is 250.
    #[test]
    fn only_hedges_are_turned() {
        let Some(res) = file() else { return };
        for t in parse_foliage(&res) {
            let hedge = t.name.contains("hedge");
            assert_eq!(t.rot_steps, if hedge { 250 } else { 1 }, "{}", t.name);
            if !hedge {
                assert!(t.instances.iter().all(|i| i.rot == 0), "{}", t.name);
            } else {
                assert!(t.instances.iter().all(|i| i.rot < 250), "{}", t.name);
            }
        }
    }
}
