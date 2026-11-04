//! Skinned characters: model loading (`resources/character/*_set`), animation clips bound
//! to a skeleton, CPU skinning into a per-frame vertex buffer. Shared by the pedestrians
//! and the drivers sitting in the cars.

use crate::assets::load_resource_file;
use crate::render::{Gpu, MatMode, Vertex};
use glam::{Mat4, Quat, Vec3};
use mcla_formats::rage::anim::{Animation, KIND_MOVER_ROTATION, KIND_MOVER_TRANSLATION, KIND_ROTATION, KIND_TRANSLATION};
use mcla_formats::rage::blendshape::BlendShapes;
use mcla_formats::rage::character::Character;
use std::path::Path;
use std::sync::Arc;

pub struct Bone {
    pub parent: Option<usize>,
    pub pos: Vec3,
    pub rot: Quat,
    /// bind pose position; the bind pose has no rotations
    pub bind: Vec3,
}

pub struct SkinVert {
    pub pos: Vec3,
    pub nrm: Vec3,
    pub uv: [f32; 2],
    pub w: [f32; 4],
    pub b: [u16; 4],
    /// the stored tangent (xyz as stored, w its sign: +1 / -1, 0 without one) and vertex colour
    /// (r the wind weight, a the occlusion the character programs read)
    pub tan: [f32; 4],
    pub col: [u8; 4],
}

pub struct Part {
    pub verts: Vec<SkinVert>,
    pub indices: Vec<u32>,
    pub tex: usize,
    /// hair cards and the like: cut out by the texture's alpha
    pub alpha: bool,
    /// its shader's character material
    pub mat: CharMat,
}

/// The character programs (characters/*.fxc, work/lighting-chars/dis/CHAR_RULE.md) as their pixel
/// programs group the twenty effects; city.wgsl reads the number from the draw's object
/// (interior[4].x). 0: none (the generic look).
pub mod fx {
    /// Character, Character_skin, Character_skin_blendshape
    pub const PLAIN: u32 = 1;
    /// Character_normalmap, Character_skin_normalmap, Character_skin_blendshape_normalmap
    pub const NM: u32 = 2;
    /// Character_alpha(_doublesided)
    pub const ALPHA: u32 = 3;
    /// Character_alpha_normalmap(_doublesided)
    pub const ALPHA_NM: u32 = 4;
    pub const TEETH: u32 = 5;
    pub const TEETH_NM: u32 = 6;
    /// Character_visor / Character_plastic (the same programs)
    pub const VISOR: u32 = 7;
    pub const VISOR_NM: u32 = 8;
    /// Character_skin_arm(_normalmap): PS_HighQualitySkin
    pub const ARM: u32 = 9;
    /// Ped (VSPedDay + PS)
    pub const PED: u32 = 10;
}

/// A part's character material: the effect, double-sided, ambientMod (VS c208) and the pixel
/// program's constants in fixed slots - k[0] (normalMapMod, detailMapUScale, detailMapVScale,
/// detailMapOffset), k[1] (diffuseMod, SpecMapExp, SpecExp, SpecIntensity), k[2] (SpecMapIntensity,
/// the fresnel's exponent, minimum, maximum), k[3] (wrapAmount, rimPower, rimAmount,
/// ambientOcclusionContrast), k[4] (envMod, metallic, -, -), k[5] tintColor.
#[derive(Clone, Copy, Default)]
pub struct CharMat {
    pub fx: u32,
    /// Character / Character_normalmap: skinned, drawn by VSSkinned_Wind (the others' VSSkinned has no wind)
    pub wind: bool,
    pub double: bool,
    pub ambient_mod: f32,
    pub k: [[f32; 4]; 6],
}

impl CharMat {
    /// From the drawable's shader: its name and its parameters by name hash.
    pub fn of(sh: &mcla_formats::rage::drawable::Shader, has_normal: bool) -> CharMat {
        use mcla_formats::rage::drawable::ShaderParam;
        let n = sh.name.to_ascii_lowercase();
        let nm = n.ends_with("_normalmap") || n.contains("_normalmap_");
        let base = n.trim_end_matches("_doublesided").trim_end_matches("_normalmap");
        let fx = match base {
            _ if n == "ped" => fx::PED,
            "character" | "character_skin" | "character_skin_blendshape" => if nm && has_normal { fx::NM } else { fx::PLAIN },
            "character_alpha" => if nm && has_normal { fx::ALPHA_NM } else { fx::ALPHA },
            "character_teeth" => if nm && has_normal { fx::TEETH_NM } else { fx::TEETH },
            "character_visor" | "character_plastic" => if nm && has_normal { fx::VISOR_NM } else { fx::VISOR },
            "character_skin_arm" if has_normal => fx::ARM,
            _ => 0,
        };
        let v = |name: &str| -> Option<f32> {
            let h = mcla_formats::hash::joaat(&name.to_ascii_lowercase());
            sh.params.iter().find_map(|(ph, p)| match p {
                ShaderParam::Vector(v) if *ph == h => v.first().map(|x| x.x),
                _ => None,
            })
        };
        let f = |a: &str| v(a).unwrap_or(0.0);
        let f2 = |a: &str, b: &str| v(a).or(v(b)).unwrap_or(0.0);
        let tint = sh.params.iter().find_map(|(ph, p)| match p {
            ShaderParam::Vector(v) if *ph == mcla_formats::hash::joaat("tintcolor") => v.first().map(|x| x.to_array()),
            _ => None,
        });
        CharMat {
            fx,
            wind: base == "character",
            double: n.ends_with("_doublesided"),
            ambient_mod: f("ambientMod"),
            k: [
                [f("normalMapMod"), f("detailMapUScale"), f("detailMapVScale"), f("detailMapOffset")],
                [f("diffuseMod"), f("SpecMapExp"), f("SpecExp"), f("SpecIntensity")],
                [f("SpecMapIntensity"), f2("specFresnelExp", "fresnelExp"), f2("specFresnelMin", "fresnelMin"), f2("specFresnelMax", "fresnelMax")],
                [f("wrapAmount"), f("rimPower"), f("rimAmount"), f("ambientOcclusionContrast")],
                [f("envMod"), f("metallic"), 0.0, 0.0],
                tint.unwrap_or([1.0; 4]),
            ],
        }
    }
}

/// VSSkinned_Wind's bend of one skinned vertex (character_programs_rule.md section 3): with the plane P and the params W,
/// sp the position in the draw's object space (here the world's less `at`, the place of its translation-only gWorld),
/// d = sp . P.xyz - P.w, ph_k = d x (30, 60, 90, 15)_k + W.x x 25 wrapped to -pi..pi, nn = norm(skinned normal),
/// a = (1 - sat(nn . P.xyz)) x W.y x the vertex colour's r: the position gains nn x a x sum sin(ph_k) (0.005, 0.0025,
/// 0.00125, 0.01)_k, the normal becomes norm(nn + P.xyz x a x sum cos(ph_k) (0.06, 0.03, 0.015, 0.12)_k). The program
/// works in its own sine / cosine; here the host's (single precision).
pub fn wind_bend(pos: Vec3, nrm: Vec3, r: u8, eq: [f32; 4], wp: [f32; 2], at: Vec3) -> (Vec3, Vec3) {
    const K: [f32; 4] = [30.0, 60.0, 90.0, 15.0];
    const S: [f32; 4] = [0.005, 0.0025, 0.00125, 0.01];
    const C: [f32; 4] = [0.06, 0.03, 0.015, 0.12];
    let pl = Vec3::new(eq[0], eq[1], eq[2]);
    let d = (pos - at).dot(pl) - eq[3];
    let nn = nrm.normalize_or(Vec3::Y);
    let a = (1.0 - nn.dot(pl).clamp(0.0, 1.0)) * wp[1] * (r as f32 / 255.0);
    let (mut s, mut c) = (0.0f32, 0.0f32);
    for k in 0..4 {
        let ph = d * K[k] + wp[0] * 25.0;
        let ph = ph - (ph / std::f32::consts::TAU).round() * std::f32::consts::TAU;
        s += ph.sin() * S[k];
        c += ph.cos() * C[k];
    }
    (pos + nn * (a * s), (nn + pl * (a * c)).normalize_or(nn))
}

/// What the story scenes' characters are lit by this frame: the light manager's state (main.rs
/// `lighting`) - the direction towards the sun (mgr + 32 negated), its colour (mgr + 64) and
/// strength (mgr + 80), the scene's ambient (mgr + 224); night: hour > 18 or < 6 (F-47292).
#[derive(Clone, Copy, Default)]
pub struct CharLight {
    pub to_light: Vec3,
    pub key: Vec3,
    pub strength: f32,
    pub amb: Vec3,
    pub night: bool,
    /// at night the lights of path A's night branch (`night_slots`) instead of the day's - the drivers, the riders
    /// and the scenes' cast
    pub night_path: bool,
    /// the light manager's dynamic list at night (the player's head lamps, the key light, the fourth record)
    pub dynamic: Option<crate::figlight::Dynamic>,
    /// the cine probe (F-22850: a scene plays): at night the cutscene light list comes before the lamp grid
    pub cine: bool,
}

/// Path A at night (F-75930 with four = 0): the three slots when nothing lights the figure - place 0, colour 0,
/// strength 1, range 10.
const NIGHT_EMPTY: ([f32; 4], [f32; 4]) = ([0.0, 0.0, 0.0, 10.0], [0.0, 0.0, 0.0, -1.0]);

impl CharLight {
    /// The three lights a character's draw gets at night (F-75930 path A, four = 0; docs/reference/lighting/
    /// character_lights_rule.md section 2, recorded on the player's rider - LIGHTING.md "Drivers and riders"):
    /// every lamp the lamp grid lists in the cells within 4 m of `p` in x and z (both grids: the cell's four,
    /// distinct, at most 32; F-79329), each scored sat(strength / d^2) x its cone (F-60154), the best four kept
    /// (F-48037); all scores 0: the slots empty. Else the best three by score fill the slots: the lamp's place,
    /// colour, strength = its score, range 1000 (a missing one empty). Then the dynamic list in order - the
    /// head lamps and the fourth record by their score, the key light by the luminance of its colour - takes
    /// slot 2 when it beats lum(slot 2's colour) x slot 2's strength: its place (the key light 100 km from the origin), its
    /// colour, strength = that score, range 1e6. With the cine probe the candidates are the cutscene light list's
    /// best four (`cutlights::best`) and the grid only when none of them scores.
    pub fn night_slots(&self, p: Vec3, t: Option<&crate::render::LampTables>) -> [[f32; 4]; 6] {
        use crate::render::{LampRec, LAMP_GRID};
        let mut cand: Vec<u16> = Vec::new();
        if let Some(t) = t {
            let axis = |v: f32, scale: f32, bias: f32, n: u32| (((v * scale + bias) * n as f32) as i64).clamp(0, n as i64 - 1) as usize;
            let (x0, x1) = (axis(p.x - 4.0, 1.0 / 6320.0, 0.54857594, LAMP_GRID.0), axis(p.x + 4.0, 1.0 / 6320.0, 0.54857594, LAMP_GRID.0));
            let (z0, z1) = (axis(p.z - 4.0, 1.0 / 4463.75, 0.5282554, LAMP_GRID.1), axis(p.z + 4.0, 1.0 / 4463.75, 0.5282554, LAMP_GRID.1));
            'cells: for z in z0..=z1 {
                for x in x0..=x1 {
                    for &i in t.cells.get(z * LAMP_GRID.0 as usize + x).unwrap_or(&[0; 4]) {
                        if !cand.contains(&i) {
                            if cand.len() == 32 {
                                break 'cells;
                            }
                            cand.push(i);
                        }
                    }
                }
            }
        }
        let rec = |i: u16| t.and_then(|t| t.recs.get(i as usize)).copied().unwrap_or_default();
        // (with the cine probe the cutscene list's best four above 1e-6 - F-65001; none: the grid as without it)
        let cut = if self.cine { crate::cutlights::LIST.get().map(|l| crate::cutlights::best(l, p)).unwrap_or_default() } else { Vec::new() };
        let scored = if !cut.is_empty() {
            cut
        } else {
            let mut scored: Vec<(f32, LampRec)> = cand.iter().map(|&i| (crate::figlight::score(&rec(i), p), rec(i))).collect();
            scored.sort_by(|a, b| b.0.total_cmp(&a.0));
            scored.truncate(4);
            scored
        };
        let mut slots = [NIGHT_EMPTY; 3];
        if scored.iter().map(|s| s.0).sum::<f32>() > 0.0 {
            for (k, (s, l)) in scored.iter().take(3).enumerate() {
                slots[k] = ([l.pos.x, l.pos.y, l.pos.z, 1000.0], [l.color.x, l.color.y, l.color.z, -s]);
            }
        }
        let lum = |c: Vec3| 0.3 * c.x + 0.59 * c.y + 0.11 * c.z;
        if let Some(d) = self.dynamic {
            // (the key light's record stands 100 km from the world's origin along its direction - the rider's
            // recording: (-67559, -73716, -1287), length 1e5)
            let key = d.key.map(|(col, dir)| LampRec { pos: dir.normalize_or(Vec3::Y) * 1.0e5, color: col, strength: 0.0, back: Vec3::ZERO, cone: [0.0, 1.0] });
            for (j, l) in [d.heads[0], d.heads[1], key, d.rec3].into_iter().enumerate() {
                let Some(l) = l else { continue };
                let s = if j == 2 { lum(l.color) } else { crate::figlight::score(&l, p) };
                let (_, c2) = slots[2];
                if s > lum(Vec3::new(c2[0], c2[1], c2[2])) * -c2[3] {
                    slots[2] = ([l.pos.x, l.pos.y, l.pos.z, 1.0e6], [l.color.x, l.color.y, l.color.z, -s]);
                }
            }
        }
        [slots[0].0, slots[1].0, slots[2].0, slots[0].1, slots[1].1, slots[2].1]
    }

    /// The three lights a story character's draw gets by day (F-21528 -> F-75930 path A,
    /// F-26319 / F-96199; work/lighting-chars/cpu/LIGHT_RULE.md, every draw of the
    /// recordings to 6.3e-8): the sun 100 m from the model's origin `p` (range 1000), colour S,
    /// w = -strength; the ambient's colour 100 m on the other side, w = -0.8 x strength
    /// (0x827E2118); the third empty (range 100). Rows: place + range, colour + (-intensity).
    pub fn day_slots(&self, p: Vec3) -> [[f32; 4]; 6] {
        let a = p + self.to_light * 100.0;
        let b = p - self.to_light * 100.0;
        [
            [a.x, a.y, a.z, 1000.0],
            [b.x, b.y, b.z, 1000.0],
            [0.0, 0.0, 0.0, 100.0],
            [self.key.x, self.key.y, self.key.z, -self.strength],
            [self.amb.x, self.amb.y, self.amb.z, -0.8 * self.strength],
            [0.0, 0.0, 0.0, -0.0],
        ]
    }
}

pub struct SkinModel {
    pub bones: Vec<Bone>,
    /// animation track id of every bone
    pub ids: Vec<u16>,
    pub lods: [Vec<Part>; 3],
    /// the cockpit view model's parts (the set's root+0x18, F-56050): arms and hands only; empty when the set has none
    pub cockpit: Vec<Part>,
    pub binds: Vec<Arc<wgpu::BindGroup>>,
    /// per shader, for the character programs: diffuse, normal map, detail map, and the spec map
    /// (alpha effects) or the normal offset map (skin_arm) - all but the diffuse uploaded as stored
    pub binds4: Vec<Arc<wgpu::BindGroup>>,
    pub mats: Vec<CharMat>,
    /// index of the head bone
    pub head: Option<usize>,
    /// the set's facial blend shapes (`.r3`), when it has targets
    pub morph: Option<Morph>,
    /// the bones of the Facial_root subtree (by the bone's place): a cine character's body request leaves them out
    /// (F-29730's filter "every bone but Facial_root and its children")
    pub face_bones: Vec<bool>,
}

/// A character set's blend shapes bound to its high-detail parts: the `.r3` mesh record names the model
/// by its place in the drawable's LOD model list (+84), its blend geometries go with that model's
/// geometries in order (docs/reference/cutscenes/blend_shapes.md).
pub struct Morph {
    pub shapes: BlendShapes,
    /// part index in `lods[0]` of every (mesh, geometry)
    pub parts: Vec<Vec<usize>>,
}

/// The creature component `crCreatureComponentBlendShapes` of one character (vtable 0x8207A4BC):
/// the weights by target (records sorted by id), the weight each target last added to the delta
/// buffer, and the buffer itself (float4 per vertex, added to the bind-pose position before skinning
/// as `VSSkinned_BlendShape` does).
pub struct FaceShapes {
    weight: Vec<f32>,
    applied: Vec<f32>,
    /// by (mesh, geometry)
    bufs: Vec<Vec<Vec<[f32; 4]>>>,
    /// updates since the last full reset (+40), and the reset waiting (+44)
    count: u32,
    reset: bool,
}

/// The component's constants (`F-75277`): every 100 updates the buffers are cleared and built
/// again (+36), a target is applied again only when its weight moved by more than 1 % (+32, 0x8207A4B0).
const RESET_EVERY: u32 = 100;
const TOLERANCE: f32 = 0.01;

impl FaceShapes {
    pub fn new(m: &Morph) -> FaceShapes {
        let n = m.shapes.targets.len();
        let bufs = m.shapes.meshes.iter().map(|me| me.geoms.iter().map(|g| vec![[0.0f32; 4]; g.vertex_count]).collect()).collect();
        FaceShapes { weight: vec![0.0; n], applied: vec![0.0; n], bufs, count: 0, reset: false }
    }

    /// Weight of target `i` (index into `Morph::shapes.targets`) as the frame's kind-21 track gives it
    /// (`F-77597`).
    pub fn set(&mut self, i: usize, w: f32) {
        self.weight[i] = w;
    }

    /// `F-15194` (the CPU path): add every changed weight's difference into the buffer.
    pub fn update(&mut self, m: &Morph) {
        self.count += 1;
        if self.count >= RESET_EVERY {
            self.reset = true;
            self.count = 0;
        }
        if self.reset {
            // F-62059 -> F-90338: the buffer is set to zero, the applied weights too
            self.bufs.iter_mut().flatten().for_each(|b| b.iter_mut().for_each(|v| *v = [0.0; 4]));
            self.applied.iter_mut().for_each(|a| *a = 0.0);
            self.reset = false;
        }
        for (i, t) in m.shapes.targets.iter().enumerate() {
            let (w, last) = (self.weight[i], self.applied[i]);
            let big = if 1.0 - w.abs() >= 0.0 { 1.0 } else { w.abs() };
            let big = if big - last.abs() >= 0.0 { big } else { last.abs() };
            let d = w - last;
            if d.abs() <= big * TOLERANCE {
                continue;
            }
            // F-63618 -> F-21008 -> F-86366 -> F-61631 on every geometry of the target
            for &(mi, gi) in &t.geoms {
                let Some(Some(blob)) = m.shapes.meshes[mi].geoms[gi].blobs.get(t.slot) else { continue };
                blob.apply(&mut self.bufs[mi][gi], d);
            }
            self.applied[i] = w;
        }
    }

    /// (targets with a weight above the tolerance, largest vertex displacement in metres) - `MCLA_BS_TRACE`
    pub fn stats(&self) -> (usize, f32) {
        let n = self.applied.iter().filter(|a| a.abs() > TOLERANCE).count();
        let d = self.bufs.iter().flatten().flatten().map(|v| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()).fold(0.0f32, f32::max);
        (n, d)
    }

    /// The delta buffer of part `p` of the high LOD, if the shapes move it.
    pub fn part(&self, m: &Morph, p: usize) -> Option<&[[f32; 4]]> {
        m.parts.iter().enumerate().find_map(|(mi, g)| g.iter().position(|&q| q == p).map(|gi| self.bufs[mi][gi].as_slice()))
    }
}

impl SkinModel {
    /// Load a character set; `name` only keys the texture cache.
    pub fn load(gpu: &mut Gpu, name: &str, path: &Path) -> Option<SkinModel> {
        Self::from_character(gpu, name, path, Self::read(path)?)
    }

    /// The character set's file, read (no GPU work: may run on a worker thread).
    pub fn read(path: &Path) -> Option<Character> {
        Character::parse(&load_resource_file(path).ok()?).ok()
    }

    /// The model of a character set read by `read`.
    pub fn from_character(gpu: &mut Gpu, name: &str, path: &Path, c: Character) -> Option<SkinModel> {
        let ids: Vec<u16> = c.bones.iter().map(|b| b.id).collect();
        let bones: Vec<Bone> = c.bones.iter().map(|b| Bone { parent: b.parent, pos: b.local_pos, rot: b.local_rot, bind: b.world_pos }).collect();
        // one texture per shader
        let mut binds = Vec::new();
        for tn in &c.shader_tex {
            let tex = c.textures.iter().find(|t| t.name.eq_ignore_ascii_case(tn)).or(c.textures.iter().find(|t| t.name.ends_with("_d.dds"))).or(c.textures.first());
            let bind = match tex {
                Some(t) => {
                    let key = mcla_formats::hash::joaat(&format!("{}/{}", name, t.name)) | 1;
                    let g = gpu.upload_texture(key, t);
                    gpu.texture_bind_group(&g, &g, key, key)
                }
                None => {
                    let w = gpu.white.clone();
                    gpu.texture_bind_group(&w, &w, 0, 0)
                }
            };
            binds.push(bind);
        }
        if binds.is_empty() {
            let w = gpu.white.clone();
            binds.push(gpu.texture_bind_group(&w, &w, 0, 0));
        }
        // the character programs' maps by parameter (NormalMapSampler / NormalSampler,
        // DetailMapSampler, SpecMapSampler / NormalOffsetSampler); a missing one is white
        let mut binds4 = Vec::new();
        let mut mats = Vec::new();
        for (si, sh) in c.shaders.iter().enumerate() {
            let find = |h: &[u32]| sh.textures().find(|(ph, _)| h.contains(ph)).and_then(|(_, n)| c.textures.iter().find(|t| t.name.eq_ignore_ascii_case(n)));
            let mut raw = |t: Option<&mcla_formats::rage::texture::Texture>| match t {
                Some(t) => {
                    let key = mcla_formats::hash::joaat(&format!("{}/raw/{}", name, t.name)) | 1;
                    (gpu.upload_texture_raw(key, t), key)
                }
                None => (gpu.white.clone(), 0),
            };
            let nrm_t = find(&[0x3350a6fd, 0x8ac11cb0]);
            let (nrm, kn) = raw(nrm_t);
            let (det, kd) = raw(find(&[0x3e19076b]));
            let (spec, ks) = raw(find(&[0x4bc6d1ed, 0x6cd2b2ab]));
            let tn = sh.diffuse().unwrap_or("");
            let dtex = c.textures.iter().find(|t| t.name.eq_ignore_ascii_case(tn)).or(c.textures.iter().find(|t| t.name.ends_with("_d.dds"))).or(c.textures.first());
            let (dif, k0) = match dtex {
                Some(t) => {
                    let key = mcla_formats::hash::joaat(&format!("{}/{}", name, t.name)) | 1;
                    (gpu.upload_texture(key, t), key)
                }
                None => (gpu.white.clone(), 0),
            };
            binds4.push(gpu.texture_bind_group4(&dif, &nrm, &det, &spec, k0, kn ^ 0x5bd1e995, kd ^ 0x27d4eb2d, ks ^ 0x165667b1));
            mats.push(CharMat::of(sh, nrm_t.is_some()));
            let _ = si;
        }
        let nb = bones.len() as u16;
        let mut lods: [Vec<Part>; 4] = Default::default();
        for (l, models) in c.lods.iter().chain(std::iter::once(&c.cockpit)).enumerate() {
            for &mi in models {
                for (mesh, si) in &c.models[mi] {
                    let mut verts = Vec::with_capacity(mesh.positions.len());
                    for (vi, p) in mesh.positions.iter().enumerate() {
                        let mut w = mesh.blend_weights.get(vi).map(|w| w.to_array()).unwrap_or([1.0, 0.0, 0.0, 0.0]);
                        let bi = mesh.blend_indices.get(vi).copied().unwrap_or([0; 4]);
                        let mut b = [0u16; 4];
                        for k in 0..4 {
                            // blend indices address the geometry's bone palette
                            let s = mesh.bone_palette.get(bi[k] as usize).copied().unwrap_or(bi[k] as u16);
                            if s >= nb {
                                w[k] = 0.0;
                            } else {
                                b[k] = s;
                            }
                        }
                        let sum: f32 = w.iter().sum();
                        if sum > 1e-4 {
                            w.iter_mut().for_each(|x| *x /= sum);
                        } else {
                            w = [1.0, 0.0, 0.0, 0.0];
                        }
                        let n = mesh.normals.get(vi).copied().unwrap_or(Vec3::Y);
                        let uv = mesh.uv0.get(vi).map(|u| [u.x, u.y]).unwrap_or([0.0; 2]);
                        // (the tangent word's sign: raw 1 = +1, raw 3 = -1 - vehicles.rs `car_binormal_mark`)
                        let tan = match mesh.tangents.get(vi) {
                            Some(t) if mesh.tangents.len() == mesh.positions.len() => [t.x, t.y, t.z, if t.w >= 1.5 { -1.0 } else if t.w < 0.5 { 0.0 } else { 1.0 }],
                            _ => [0.0; 4],
                        };
                        let col = mesh.colors.get(vi).copied().unwrap_or([255; 4]);
                        verts.push(SkinVert { pos: *p, nrm: if n.is_finite() { n } else { Vec3::Y }, uv, w, b, tan, col });
                    }
                    let alpha = c.shader_names.get(*si as usize).map_or(false, |n| n.to_ascii_lowercase().contains("alpha"));
                    let mat = mats.get(*si as usize).copied().unwrap_or_default();
                    lods[l].push(Part { verts, indices: mesh.indices.clone(), tex: (*si as usize).min(binds.len() - 1), alpha, mat });
                }
            }
        }
        if lods[0].is_empty() {
            return None;
        }
        let [l0, l1, l2, cockpit] = lods;
        let lods = [l0, l1, l2];
        let morph = load_morph(path, &c, &lods[0]);
        if binds4.is_empty() {
            binds4 = binds.clone();
        }
        let facial = c.bone("Facial_root");
        let face_bones = (0..bones.len())
            .map(|i| {
                let mut b = Some(i);
                while let Some(j) = b {
                    if Some(j) == facial {
                        return true;
                    }
                    b = bones[j].parent;
                }
                false
            })
            .collect();
        Some(SkinModel { bones, ids, lods, cockpit, binds, binds4, mats, head: c.bone("head"), morph, face_bones })
    }
}

/// The `.r3` next to a set's `.r6` (the cine spawner `F-62204` loads it when it exists), bound to
/// the high LOD's parts. Sets with an empty manager (231 of 247) have none.
fn load_morph(path: &Path, c: &Character, high: &[Part]) -> Option<Morph> {
    let dir = path.parent()?;
    let r3 = std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).find(|p| p.extension().map_or(false, |e| e.eq_ignore_ascii_case("r3")))?;
    let shapes = match BlendShapes::parse(&load_resource_file(&r3).ok()?) {
        Ok(s) => s,
        Err(e) => {
            log::warn!("blend shapes {}: {}", r3.display(), e);
            return None;
        }
    };
    if shapes.targets.is_empty() {
        return None;
    }
    let mut parts = Vec::new();
    for me in &shapes.meshes {
        // every file names LOD 0; the port's parts of other LODs are not laid out for this
        if me.lod != 0 {
            log::warn!("blend shapes {}: {} on LOD {}", r3.display(), me.name, me.lod);
            return None;
        }
        let k = me.model as usize;
        let mi = *c.lods[0].get(k)?;
        // parts are laid out model by model in the LOD's order, one per geometry
        let first: usize = c.lods[0][..k].iter().map(|&x| c.models[x].len()).sum();
        let mut g = Vec::new();
        for (gi, bg) in me.geoms.iter().enumerate() {
            let p = first + gi;
            if gi >= c.models[mi].len() || high.get(p).map_or(true, |part| part.verts.len() != bg.vertex_count) {
                log::warn!("blend shapes {}: {} geometry {} does not fit the drawable", r3.display(), me.name, gi);
                return None;
            }
            g.push(p);
        }
        parts.push(g);
    }
    Some(Morph { shapes, parts })
}

/// An animation bound to one skeleton: track index per bone.
pub struct Clip {
    pub anim: Arc<Animation>,
    pub rot: Vec<Option<usize>>,
    pub pos: Vec<Option<usize>>,
    pub mover_t: Option<usize>,
    pub mover_r: Option<usize>,
    /// "Fast" evades use the quick speech lines
    pub fast: bool,
    /// only when the car comes from ahead
    pub front: bool,
    /// model-space displacement over the whole clip
    pub travel: Vec3,
}

impl Clip {
    pub fn bind(anim: &Arc<Animation>, bones: &[u16], fast: bool, front: bool) -> Clip {
        let find = |kind: u8, id: u16| anim.tracks.iter().position(|t| t.kind == kind && t.bone_id == id);
        let mover_t = find(KIND_MOVER_TRANSLATION, 0);
        let travel = mover_t.map(|t| anim.tracks[t].data.vec((anim.frames.max(1) - 1) as f32) - anim.tracks[t].data.vec(0.0)).unwrap_or(Vec3::ZERO);
        Clip {
            anim: anim.clone(),
            rot: bones.iter().map(|&id| find(KIND_ROTATION, id)).collect(),
            pos: bones.iter().map(|&id| find(KIND_TRANSLATION, id)).collect(),
            mover_t,
            mover_r: find(KIND_MOVER_ROTATION, 0),
            fast,
            front,
            travel,
        }
    }

    pub fn last_frame(&self) -> f32 {
        (self.anim.frames.max(1) - 1) as f32
    }

    pub fn frame(&self, time: f32, looped: bool) -> f32 {
        let last = self.last_frame();
        if last <= 0.0 || self.anim.duration <= 0.0 {
            return 0.0;
        }
        let f = time / self.anim.duration * last;
        if looped {
            f.rem_euclid(last)
        } else {
            f.clamp(0.0, last)
        }
    }

    pub fn mover(&self, f: f32) -> (Vec3, Quat) {
        (
            self.mover_t.map(|t| self.anim.tracks[t].data.vec(f)).unwrap_or(Vec3::ZERO),
            self.mover_r.map(|t| self.anim.tracks[t].data.rot(f)).unwrap_or(Quat::IDENTITY),
        )
    }

    /// Local pose of every bone at frame `f` (bones without a track keep their rest pose).
    pub fn local(&self, bones: &[Bone], f: f32, out: &mut Vec<(Quat, Vec3)>) {
        out.clear();
        for (i, b) in bones.iter().enumerate() {
            let r = self.rot[i].map(|t| self.anim.tracks[t].data.rot(f)).unwrap_or(b.rot);
            let p = self.pos[i].map(|t| self.anim.tracks[t].data.vec(f)).unwrap_or(b.pos);
            out.push((r, p));
        }
    }
}

/// Blend pose `b` into `a` with weight `k` (0 keeps `a`).
pub fn blend_pose(a: &mut [(Quat, Vec3)], b: &[(Quat, Vec3)], k: f32) {
    for (l, f) in a.iter_mut().zip(b.iter()) {
        let fr = if l.0.dot(f.0) < 0.0 { -f.0 } else { f.0 };
        l.0 = l.0.lerp(fr, k).normalize();
        l.1 = l.1.lerp(f.1, k);
    }
}

/// This frame's skinned geometry of one kind of character.
#[derive(Default)]
pub struct SkinBuffer {
    verts: Vec<Vertex>,
    indices: Vec<u32>,
    draws: Vec<(Arc<wgpu::BindGroup>, u32, u32)>,
    vb: Option<wgpu::Buffer>,
    ib: Option<wgpu::Buffer>,
    vb_cap: usize,
    ib_cap: usize,
    world: Vec<Mat4>,
    skin: Vec<Mat4>,
    /// the next figure is drawn without this bone and what hangs on it (the driver's head
    /// in the cockpit view: its vertices are gathered in the joint)
    pub hide_from: Option<usize>,
    /// the next `emit` draws the model's cockpit view model instead of its LODs (when it has one; F-56050)
    pub next_cockpit: bool,
    hidden: Vec<bool>,
    /// the next figure is lit by these four lanes (a walker at night - figlight.rs); taken by `emit`
    pub next_lanes: Option<[[f32; 4]; 12]>,
    /// this frame's lit figures' lanes, the figure of each draw (usize::MAX: none) and the
    /// object uniforms that carry them (one per lit figure, kept from frame to frame)
    lanes: Vec<[[f32; 4]; 12]>,
    draw_fig: Vec<usize>,
    lane_objs: Vec<crate::render::ObjectBinding>,
    /// Some: this buffer's figures are drawn with the character programs (the story scenes - cine.rs),
    /// lit as the light manager stands this frame
    pub char_light: Option<CharLight>,
    /// the next figure is one of a scene's bystanders (an entity named "ped": F-65008 sets
    /// +149, the draw takes the manager's own lights - path B); taken by `emit`
    pub next_bystander: bool,
    /// the stencil reference the next figure draws with (a car's driver: render/mb.rs `driver_stencil_of`; 0 the
    /// world's); taken by `emit`
    pub next_stencil: u32,
    /// the next figure's bones in the world as given (the thrown rider's from his ragdoll, F-46606) instead of
    /// its local pose; taken by `emit`
    pub next_world: Option<Vec<Mat4>>,
    /// the lights' origin is the matrix `emit` is given (a driver's: his car's or bike's - the draw's gWorld row 3 in the
    /// rider's recording), not the root bone's place (the scenes' characters)
    pub light_at_root: bool,
    /// the next figure sits in a car: its gauge / neon lights and floor (drivers.rs `Cabin`); taken by `emit`
    pub next_cabin: Option<crate::drivers::Cabin>,
    /// each draw's stencil reference
    draw_stencil: Vec<u32>,
    /// this frame's figures (character programs): the model's origin, bystander, the car's lights
    figs: Vec<(Vec3, bool, Option<crate::drivers::Cabin>)>,
    /// and each draw's figure and material
    char_draws: Vec<(usize, CharMat)>,
    char_objs: Vec<crate::render::ObjectBinding>,
    /// the objects' uniforms of this frame, written in one upload
    uniforms: crate::render::UniformBatch,
}

impl SkinBuffer {
    pub fn clear(&mut self) {
        self.verts.clear();
        self.indices.clear();
        self.draws.clear();
        self.lanes.clear();
        self.draw_fig.clear();
        self.figs.clear();
        self.char_draws.clear();
        self.draw_stencil.clear();
    }

    /// Skin one character in the given local pose, standing at `root`.
    pub fn emit(&mut self, model: &SkinModel, lod: usize, local: &[(Quat, Vec3)], root: Mat4) {
        self.emit_shaped(model, lod, local, root, None);
    }

    /// The same with the face's blend shapes: their delta is added to the bind-pose position before
    /// skinning (`VSSkinned_BlendShape` 17: `r2.xyz = add(r2, r0)`; the normal stays).
    pub fn emit_shaped(&mut self, model: &SkinModel, lod: usize, local: &[(Quat, Vec3)], root: Mat4, face: Option<&FaceShapes>) {
        self.world.clear();
        self.skin.clear();
        let given = self.next_world.take().filter(|g| g.len() >= model.bones.len());
        for (i, b) in model.bones.iter().enumerate() {
            let w = match (&given, b.parent) {
                (Some(g), _) => g[i],
                (None, Some(pi)) if pi < i => self.world[pi] * Mat4::from_rotation_translation(local[i].0, local[i].1),
                _ => root * Mat4::from_rotation_translation(local[i].0, local[i].1),
            };
            self.world.push(w);
            let gone = self.hide_from == Some(i) || matches!(b.parent, Some(pi) if pi < i && self.hidden.get(pi).copied().unwrap_or(false));
            if i == 0 {
                self.hidden.clear();
            }
            self.hidden.push(gone);
            // the bind pose has no rotations: its inverse is a plain translation
            self.skin.push(if gone { Mat4::from_translation(w.w_axis.truncate()) * Mat4::from_scale(Vec3::ZERO) } else { w * Mat4::from_translation(-b.bind) });
        }
        self.hide_from = None;
        let stencil = std::mem::take(&mut self.next_stencil);
        let fig = match self.next_lanes.take() {
            Some(l) => {
                self.lanes.push(l);
                self.lanes.len() - 1
            }
            None => usize::MAX,
        };
        let first = (lod.min(2)..3).find(|&l| !model.lods[l].is_empty()).unwrap_or(0);
        let parts = if std::mem::take(&mut self.next_cockpit) && !model.cockpit.is_empty() { &model.cockpit } else { &model.lods[first] };
        let chars = self.char_light.is_some();
        let cfig = self.figs.len();
        let cabin = self.next_cabin.take();
        if chars {
            // (the lights' origin: the model's place, gWorld row 3 - here the root bone's, which carries
            // the scene position (cine.rs); x / z within 3 cm of the recorded draws, y up to 0.4 m higher)
            let at = if self.light_at_root { root.w_axis } else { self.world.first().map_or(root.w_axis, |w| w.w_axis) };
            self.figs.push((at.truncate(), std::mem::take(&mut self.next_bystander), cabin));
        }
        // (a driver's wind: windEmitterPlaneEq, windParams and the draw's place - its gWorld is a translation alone, so
        // the program's object space is the world's less that place; MCLA_DRV_WIND_OLD=1: none)
        let wind = cabin.filter(|c| c.wind[1] != 0.0 && std::env::var("MCLA_DRV_WIND_OLD").is_err()).map(|c| (c.wind_eq, c.wind, root.w_axis.truncate()));
        // (the night fill light at the figure's place - fillmap.rs: the figures share one buffer
        // and the identity object, so the colour rides in every vertex: color1.rgb = four times
        // map / 255 x 0.25 x m_NFLFade, uv1.y = 77 the mark city.wgsl knows a figure by)
        let fill = crate::fillmap::figure_now(root.w_axis.x, root.w_axis.z);
        for (pi, part) in parts.iter().enumerate() {
            let base = self.verts.len() as u32;
            let shape = match (face, model.morph.as_ref()) {
                (Some(f), Some(m)) if first == 0 => f.part(m, pi),
                _ => None,
            };
            for (vi, v) in part.verts.iter().enumerate() {
                let bind = match shape {
                    Some(d) => v.pos + Vec3::new(d[vi][0], d[vi][1], d[vi][2]),
                    None => v.pos,
                };
                let mut pos = Vec3::ZERO;
                let mut nrm = Vec3::ZERO;
                let mut tan = Vec3::ZERO;
                let t = Vec3::new(v.tan[0], v.tan[1], v.tan[2]);
                for k in 0..4 {
                    if v.w[k] > 0.0 {
                        let m = &self.skin[v.b[k] as usize];
                        pos += m.transform_point3(bind) * v.w[k];
                        nrm += m.transform_vector3(v.nrm) * v.w[k];
                        tan += m.transform_vector3(t) * v.w[k];
                    }
                }
                if let (true, Some((eq, wp, at))) = (chars && part.mat.wind, wind) {
                    let (p2, n2) = wind_bend(pos, nrm, v.col[0], eq, wp, at);
                    pos = p2;
                    nrm = n2;
                }
                if chars {
                    // (the character programs: the skinned tangent - normalised after the world
                    // matrix as VSSkinned does - octahedral in uv1.x / uv2.y, its sign as + 4 on
                    // uv1.x (no sign: + 8), unpacked by city.wgsl `vs_body` into `fill`; the vertex
                    // colour as stored)
                    let o = oct(tan.normalize_or(Vec3::X));
                    let sgn = if v.tan[3] < -0.5 { 4.0 } else if v.tan[3] < 0.5 { 8.0 } else { 0.0 };
                    self.verts.push(Vertex { pos: pos.to_array(), normal: nrm.normalize_or(Vec3::Y).to_array(), uv0: v.uv, uv1: [o[0] + sgn, 77.0], color: v.col, color1: fill, uv2: [if part.alpha { 1.0 } else { 0.0 }, o[1]] });
                    continue;
                }
                self.verts.push(Vertex { pos: pos.to_array(), normal: nrm.normalize_or(Vec3::Y).to_array(), uv0: v.uv, uv1: [0.0, 77.0], color: [255; 4], color1: fill, uv2: [if part.alpha { 1.0 } else { 0.0 }, 0.0] });
            }
            let first = self.indices.len() as u32;
            self.indices.extend(part.indices.iter().map(|i| i + base));
            if chars && part.mat.fx != 0 {
                self.draws.push((model.binds4.get(part.tex).unwrap_or(&model.binds[part.tex]).clone(), first, part.indices.len() as u32));
                self.char_draws.push((cfig, part.mat));
            } else {
                self.draws.push((model.binds[part.tex].clone(), first, part.indices.len() as u32));
                if chars {
                    self.char_draws.push((cfig, CharMat::default()));
                }
            }
            self.draw_fig.push(fig);
            self.draw_stencil.push(stencil);
        }
    }

    /// World position of a bone of the character emitted last.
    pub fn bone_pos(&self, bone: usize) -> Option<Vec3> {
        self.world.get(bone).map(|m| m.w_axis.truncate())
    }

    pub fn upload(&mut self, gpu: &Gpu) {
        if self.verts.is_empty() {
            return;
        }
        if self.vb.is_none() || self.vb_cap < self.verts.len() {
            self.vb_cap = (self.verts.len() * 3 / 2).max(16384);
            self.vb = Some(gpu.device.create_buffer(&wgpu::BufferDescriptor { label: Some("skin vb"), size: (self.vb_cap * std::mem::size_of::<Vertex>()) as u64, usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false }));
        }
        if self.ib.is_none() || self.ib_cap < self.indices.len() {
            self.ib_cap = (self.indices.len() * 3 / 2).max(65536);
            self.ib = Some(gpu.device.create_buffer(&wgpu::BufferDescriptor { label: Some("skin ib"), size: (self.ib_cap * 4) as u64, usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false }));
        }
        gpu.queue.write_buffer(self.vb.as_ref().unwrap(), 0, bytemuck::cast_slice(&self.verts));
        gpu.queue.write_buffer(self.ib.as_ref().unwrap(), 0, bytemuck::cast_slice(&self.indices));
        // (the lit figures' lanes: rows 0..11 of the object's first lamp table, the mark 2 in row 0
        // of the second - city.wgsl, the walkers at night; a traffic car's row 0 there is <= 1)
        // (the character programs: one object a draw - city.wgsl reads the three lights from
        // area_a / area_b [0..2], ambYPos and ambientMod from area_a[3], floorPlaneEq from
        // area_b[3], the material's slots from area_a[4], area_b[4], interior[0..3] and
        // (effect, shadow source, double-sided, bystander) from interior[4]. The scenes' draws
        // read the full-screen collector: ShaderGlobals.z = 8.09 >= 8 in every recorded draw.)
        if let Some(cl) = self.char_light {
            let night = cl.night && cl.night_path;
            let tables = if night { crate::render::LAMP_TABLES.read().ok().and_then(|t| t.clone()) } else { None };
            for (i, (f, m)) in self.char_draws.iter().enumerate() {
                let (p, by, cabin) = self.figs[*f];
                let mut u = crate::render::ObjectUniform::default();
                let sl = if night { cl.night_slots(p, tables.as_deref()) } else { cl.day_slots(p) };
                for k in 0..3 {
                    u.area_a[k] = sl[k];
                    u.area_b[k] = sl[3 + k];
                }
                // (ambYPos: the scene's ambient by day; at night the ambient cube's faces are the fill map's
                // colour, map / 255 x 0.25 with no fade - F-35455, recorded on the rider)
                let amb = if night { crate::fillmap::cube_at(p.x, p.z) } else { cl.amb };
                u.area_a[3] = [amb.x, amb.y, amb.z, m.ambient_mod];
                // (F-42959: outside a car the neon and gauge light colours 0 and the floor far below; a driver gets
                // his car's - floorPlaneEq in area_b[3], gauge place / colour / cone direction and neon place / colour in
                // lamp_a[14..18], w of [14] = 1 marking them for city.wgsl `ch_vlights`)
                u.area_b[3] = [0.0, 1.0, 0.0, -1000.0];
                if let Some(c) = cabin {
                    u.area_b[3] = c.floor;
                    u.lamp_a[14] = c.gauge_pos.extend(1.0).to_array();
                    u.lamp_a[15] = c.gauge_col.extend(1.0).to_array();
                    u.lamp_a[16] = c.cone_dir.extend(0.0).to_array();
                    u.lamp_a[17] = c.neon_pos.extend(1.0).to_array();
                    u.lamp_a[18] = c.neon_col.extend(1.0).to_array();
                }
                u.area_a[4] = m.k[0];
                u.area_b[4] = m.k[1];
                u.interior[0] = m.k[2];
                u.interior[1] = m.k[3];
                u.interior[2] = m.k[4];
                u.interior[3] = m.k[5];
                // (a Ped program at night is VSPedNight + PSMultiLight with the walkers' four lanes -
                // not kept for the scenes' bystanders: the generic look then)
                let fxv = if cl.night && m.fx == fx::PED { 0 } else { m.fx };
                u.interior[4] = [fxv as f32, 1.0, if m.double { 1.0 } else { 0.0 }, if by { 1.0 } else { 0.0 }];
                if i < self.char_objs.len() {
                    self.uniforms.push_object(&self.char_objs[i], &u);
                } else {
                    self.char_objs.push(gpu.new_object(&u));
                }
            }
        }
        for (i, l) in self.lanes.iter().enumerate() {
            let mut u = crate::render::ObjectUniform::default();
            u.lamp_a[..12].copy_from_slice(l);
            u.lamp_b[0][3] = 2.0;
            if i < self.lane_objs.len() {
                self.uniforms.push_object(&self.lane_objs[i], &u);
            } else {
                self.lane_objs.push(gpu.new_object(&u));
            }
        }
        gpu.flush_uniforms(&mut self.uniforms);
    }

    /// What a shadow pass draws of it (`draw` with `shadow`), kept apart so the next frame's skinning can run while
    /// the shadow passes are recorded (main.rs: the same buffers, bind and count they recorded before).
    pub fn shadow_part(&self) -> Option<ShadowSkin> {
        let (Some(vb), Some(ib)) = (self.vb.as_ref(), self.ib.as_ref()) else { return None };
        let bind = self.draws.first()?.0.clone();
        Some(ShadowSkin { vb: vb.clone(), ib: ib.clone(), bind, count: self.indices.len() as u32 })
    }

    /// Draw with the pipeline already set (opaque pass or shadow pass).
    pub fn draw<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>, gpu: &'a Gpu, shadow: bool, draws: &mut usize) {
        let (Some(vb), Some(ib)) = (self.vb.as_ref(), self.ib.as_ref()) else { return };
        if self.draws.is_empty() {
            return;
        }
        pass.set_bind_group(3, &gpu.identity_obj.bg, &[]);
        pass.set_vertex_buffer(0, vb.slice(..));
        pass.set_index_buffer(ib.slice(..), wgpu::IndexFormat::Uint32);
        if shadow {
            pass.set_bind_group(1, self.draws[0].0.as_ref(), &[]);
            pass.set_bind_group(2, &gpu.mat_bgs[&(MatMode::Opaque as u32)], &[]);
            pass.draw_indexed(0..self.indices.len() as u32, 0, 0..1);
            *draws += 1;
            return;
        }
        pass.set_bind_group(2, &gpu.mat_bgs[&(MatMode::Object as u32)], &[]);
        let mut last: *const wgpu::BindGroup = std::ptr::null();
        let mut obj = usize::MAX;
        // (the stencil reference as the pass has it - 0, the world's - changed only for the figures that carry one)
        let mut stencil = 0u32;
        for (k, (bind, first, count)) in self.draws.iter().enumerate() {
            let st = self.draw_stencil.get(k).copied().unwrap_or(0);
            if st != stencil {
                pass.set_stencil_reference(st);
                stencil = st;
            }
            if Arc::as_ptr(bind) != last {
                pass.set_bind_group(1, bind.as_ref(), &[]);
                last = Arc::as_ptr(bind);
            }
            if self.char_light.is_some() {
                // (one object a draw)
                pass.set_bind_group(3, self.char_objs.get(k).map_or(&gpu.identity_obj.bg, |o| &o.bg), &[]);
                pass.draw_indexed(*first..*first + *count, 0, 0..1);
                *draws += 1;
                continue;
            }
            let fig = self.draw_fig.get(k).copied().unwrap_or(usize::MAX);
            if fig != obj {
                pass.set_bind_group(3, self.lane_objs.get(fig).map_or(&gpu.identity_obj.bg, |o| &o.bg), &[]);
                obj = fig;
            }
            pass.draw_indexed(*first..*first + *count, 0, 0..1);
            *draws += 1;
        }
        if stencil != 0 {
            pass.set_stencil_reference(0);
        }
    }
}

/// A skin buffer's shadow draw (`SkinBuffer::shadow_part`).
pub struct ShadowSkin {
    vb: wgpu::Buffer,
    ib: wgpu::Buffer,
    bind: Arc<wgpu::BindGroup>,
    count: u32,
}

impl ShadowSkin {
    /// `SkinBuffer::draw` with `shadow`: with the pipeline already set.
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>, gpu: &Gpu, draws: &mut usize) {
        pass.set_bind_group(3, &gpu.identity_obj.bg, &[]);
        pass.set_vertex_buffer(0, self.vb.slice(..));
        pass.set_index_buffer(self.ib.slice(..), wgpu::IndexFormat::Uint32);
        pass.set_bind_group(1, self.bind.as_ref(), &[]);
        pass.set_bind_group(2, &gpu.mat_bgs[&(MatMode::Opaque as u32)], &[]);
        pass.draw_indexed(0..self.count, 0, 0..1);
        *draws += 1;
    }
}

/// Octahedral encoding of a unit vector (city.wgsl `oct_dec`).
fn oct(v: Vec3) -> [f32; 2] {
    let n = v / (v.x.abs() + v.y.abs() + v.z.abs());
    if n.z >= 0.0 {
        [n.x, n.y]
    } else {
        [(1.0 - n.y.abs()) * n.x.signum(), (1.0 - n.x.abs()) * n.y.signum()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// g.28.bin draw 1457 (GH_06, 17:00): the model at c3, the lights c16..c18 / c39..c41.
    #[test]
    fn day_slots_match_the_recorded_draw() {
        let p = Vec3::new(-179.41, 18.178, -751.71);
        let cl = CharLight {
            to_light: (Vec3::new(-268.15, 64.034, -747.02) - p) / 100.0,
            key: Vec3::new(1.0, 0.47486, 0.097636),
            strength: 1.0,
            amb: Vec3::new(0.10388, 0.11954, 0.14902),
            night: false,
            night_path: false,
            dynamic: None,
            cine: false,
        };
        let want = [
            [-268.15, 64.034, -747.02, 1000.0],
            [-90.668, -27.678, -756.4, 1000.0],
            [0.0, 0.0, 0.0, 100.0],
            [1.0, 0.47486, 0.097636, -1.0],
            [0.10388, 0.11954, 0.14902, -0.8],
            [0.0, 0.0, 0.0, -0.0],
        ];
        for (r, w) in cl.day_slots(p).iter().zip(want.iter()) {
            for k in 0..4 {
                assert!((r[k] - w[k]).abs() < 0.01, "{r:?} vs {w:?}");
            }
        }
    }

    /// Path A at night on the player's rider (work/police17/cap/rider_night.bin draws 627 - 638, 23:00, at
    /// (819.79, 17.583, -808.43)): the two lamps the grid lists there, best first, strength = their score, and the key
    /// light in slot 2 at 1e5 along its direction from the origin; nothing near: the empty slots.
    #[test]
    fn night_slots_match_the_riders_draw() {
        use crate::render::{lamp_cell_index, LampRec, LampTables, LAMP_GRID};
        let p = Vec3::new(819.79, 17.583, -808.43);
        // (the recorded lamps with strengths that give the recorded scores at p, all round)
        let lamp = |pos: Vec3, color: Vec3, score: f32| LampRec { pos, color, strength: score * (pos - p).length_squared(), back: Vec3::ZERO, cone: [0.0, 1.0] };
        let a = lamp(Vec3::new(817.34, 26.321, -805.26), Vec3::new(1.0, 0.80566, 0.33276), 0.43313);
        let b = lamp(Vec3::new(830.36, 27.525, -797.77), Vec3::new(1.0, 0.75, 0.5498), 0.055978);
        let mut cells = vec![[0u16; 4]; (LAMP_GRID.0 * LAMP_GRID.1) as usize];
        cells[lamp_cell_index(p)] = [2, 1, 0, 0];
        let t = LampTables { cells, recs: vec![LampRec::default(), a, b] };
        let key = Vec3::new(-67559.0, -73716.0, -1286.7);
        let cl = CharLight {
            night: true,
            night_path: true,
            dynamic: Some(crate::figlight::Dynamic { heads: [None; 2], key: Some((Vec3::splat(0.05098), key / 1.0e5)), rec3: None }),
            ..Default::default()
        };
        let want = [
            [817.34, 26.321, -805.26, 1000.0],
            [830.36, 27.525, -797.77, 1000.0],
            [-67559.0, -73716.0, -1286.7, 1.0e6],
            [1.0, 0.80566, 0.33276, -0.43313],
            [1.0, 0.75, 0.5498, -0.055978],
            [0.05098, 0.05098, 0.05098, -0.05098],
        ];
        for (r, w) in cl.night_slots(p, Some(&t)).iter().zip(want.iter()) {
            for k in 0..4 {
                assert!((r[k] - w[k]).abs() <= 1e-4 * w[k].abs().max(1.0), "{r:?} vs {w:?}");
            }
        }
        // (no lamp within 4 m and no dynamic list: place 0, colour 0, strength 1, range 10)
        let empty = CharLight { dynamic: None, ..cl }.night_slots(Vec3::new(-3000.0, 0.0, 2000.0), Some(&t));
        assert_eq!(empty[0], [0.0, 0.0, 0.0, 10.0]);
        assert_eq!(empty[5], [0.0, 0.0, 0.0, -1.0]);
    }

    /// The player's driver sets carry the cockpit view model at root+0x18 (F-56050 draws it in the POV view):
    /// skinned to the arms and hands only - no head, neck or spine - next to a full main model.
    #[test]
    fn player_set_has_an_arms_only_cockpit_model() {
        let data = std::env::var("MCLA_DATA_DIR").unwrap_or_else(|_| r"D:\MCLA\extracted\cache".into());
        let dir = std::path::Path::new(&data).join("resources").join("character");
        for set in ["drv_mp_01_set_h", "drv_mp_01_mot_set_h"] {
            let Some(f) = std::fs::read_dir(dir.join(set)).ok().and_then(|d| d.flatten().map(|e| e.path()).find(|p| p.extension().is_some_and(|e| e == "r6"))) else { return };
            let c = SkinModel::read(&f).expect(set);
            assert!(!c.cockpit.is_empty(), "{set}: no cockpit model");
            let used = |models: &[usize]| {
                let mut b = std::collections::BTreeSet::new();
                for &mi in models {
                    for (m, _) in &c.models[mi] {
                        for (vi, ix) in m.blend_indices.iter().enumerate() {
                            let w = m.blend_weights.get(vi).map(|w| w.to_array()).unwrap_or([1.0, 0.0, 0.0, 0.0]);
                            for k in 0..4 {
                                if w[k] > 0.0 {
                                    let bone = m.bone_palette.get(ix[k] as usize).copied().unwrap_or(ix[k] as u16) as usize;
                                    b.insert(c.bones.get(bone).map_or(String::new(), |x| x.name.to_ascii_lowercase()));
                                }
                            }
                        }
                    }
                }
                b
            };
            let (cock, main) = (used(&c.cockpit), used(&c.lods[0]));
            assert!(cock.iter().all(|n| !n.contains("head") && !n.contains("neck") && !n.contains("spine")), "{set}: {cock:?}");
            assert!(main.iter().any(|n| n.contains("head")), "{set}: {main:?}");
            assert!(c.cockpit.iter().all(|i| !c.lods[0].contains(i)), "{set}");
        }
    }
}
