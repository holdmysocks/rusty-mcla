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
}

pub struct Part {
    pub verts: Vec<SkinVert>,
    pub indices: Vec<u32>,
    pub tex: usize,
    /// hair cards and the like: cut out by the texture's alpha
    pub alpha: bool,
}

pub struct SkinModel {
    pub bones: Vec<Bone>,
    /// animation track id of every bone
    pub ids: Vec<u16>,
    pub lods: [Vec<Part>; 3],
    pub binds: Vec<Arc<wgpu::BindGroup>>,
    /// index of the head bone
    pub head: Option<usize>,
    /// the set's facial blend shapes (`.r3`), when it has targets
    pub morph: Option<Morph>,
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

/// The component's constants (`sub_8253F7E0`): every 100 updates the buffers are cleared and built
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
    /// (`sub_8253EE48`).
    pub fn set(&mut self, i: usize, w: f32) {
        self.weight[i] = w;
    }

    /// `sub_8253F030` (the CPU path): add every changed weight's difference into the buffer.
    pub fn update(&mut self, m: &Morph) {
        self.count += 1;
        if self.count >= RESET_EVERY {
            self.reset = true;
            self.count = 0;
        }
        if self.reset {
            // sub_82613D68 -> sub_821B4260: the buffer is set to zero, the applied weights too
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
            // sub_82613FC8 -> sub_82615520 -> sub_826157B8 -> sub_82615F78 on every geometry of the target
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
        let c = Character::parse(&load_resource_file(path).ok()?).ok()?;
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
        let nb = bones.len() as u16;
        let mut lods: [Vec<Part>; 3] = Default::default();
        for (l, models) in c.lods.iter().enumerate() {
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
                        verts.push(SkinVert { pos: *p, nrm: if n.is_finite() { n } else { Vec3::Y }, uv, w, b });
                    }
                    let alpha = c.shader_names.get(*si as usize).map_or(false, |n| n.to_ascii_lowercase().contains("alpha"));
                    lods[l].push(Part { verts, indices: mesh.indices.clone(), tex: (*si as usize).min(binds.len() - 1), alpha });
                }
            }
        }
        if lods[0].is_empty() {
            return None;
        }
        let morph = load_morph(path, &c, &lods[0]);
        Some(SkinModel { bones, ids, lods, binds, head: c.bone("head"), morph })
    }
}

/// The `.r3` next to a set's `.r6` (the cine spawner `sub_823D2AB0` loads it when it exists), bound to
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
    hidden: Vec<bool>,
    /// the next figure is lit by these four lanes (a walker at night - figlight.rs); taken by `emit`
    pub next_lanes: Option<[[f32; 4]; 12]>,
    /// this frame's lit figures' lanes, the figure of each draw (usize::MAX: none) and the
    /// object uniforms that carry them (one per lit figure, kept from frame to frame)
    lanes: Vec<[[f32; 4]; 12]>,
    draw_fig: Vec<usize>,
    lane_objs: Vec<crate::render::ObjectBinding>,
}

impl SkinBuffer {
    pub fn clear(&mut self) {
        self.verts.clear();
        self.indices.clear();
        self.draws.clear();
        self.lanes.clear();
        self.draw_fig.clear();
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
        for (i, b) in model.bones.iter().enumerate() {
            let m = Mat4::from_rotation_translation(local[i].0, local[i].1);
            let w = match b.parent {
                Some(pi) if pi < i => self.world[pi] * m,
                _ => root * m,
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
        let fig = match self.next_lanes.take() {
            Some(l) => {
                self.lanes.push(l);
                self.lanes.len() - 1
            }
            None => usize::MAX,
        };
        let first = (lod.min(2)..3).find(|&l| !model.lods[l].is_empty()).unwrap_or(0);
        let parts = &model.lods[first];
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
                for k in 0..4 {
                    if v.w[k] > 0.0 {
                        let m = &self.skin[v.b[k] as usize];
                        pos += m.transform_point3(bind) * v.w[k];
                        nrm += m.transform_vector3(v.nrm) * v.w[k];
                    }
                }
                self.verts.push(Vertex { pos: pos.to_array(), normal: nrm.normalize_or(Vec3::Y).to_array(), uv0: v.uv, uv1: [0.0, 77.0], color: [255; 4], color1: fill, uv2: [if part.alpha { 1.0 } else { 0.0 }, 0.0] });
            }
            let first = self.indices.len() as u32;
            self.indices.extend(part.indices.iter().map(|i| i + base));
            self.draws.push((model.binds[part.tex].clone(), first, part.indices.len() as u32));
            self.draw_fig.push(fig);
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
        for (i, l) in self.lanes.iter().enumerate() {
            let mut u = crate::render::ObjectUniform::default();
            u.lamp_a[..12].copy_from_slice(l);
            u.lamp_b[0][3] = 2.0;
            if i < self.lane_objs.len() {
                gpu.queue.write_buffer(&self.lane_objs[i].buf, 0, bytemuck::bytes_of(&u));
            } else {
                self.lane_objs.push(gpu.new_object(&u));
            }
        }
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
        for (k, (bind, first, count)) in self.draws.iter().enumerate() {
            if Arc::as_ptr(bind) != last {
                pass.set_bind_group(1, bind.as_ref(), &[]);
                last = Arc::as_ptr(bind);
            }
            let fig = self.draw_fig.get(k).copied().unwrap_or(usize::MAX);
            if fig != obj {
                pass.set_bind_group(3, self.lane_objs.get(fig).map_or(&gpu.identity_obj.bg, |o| &o.bg), &[]);
                obj = fig;
            }
            pass.draw_indexed(*first..*first + *count, 0, 0..1);
            *draws += 1;
        }
    }
}
