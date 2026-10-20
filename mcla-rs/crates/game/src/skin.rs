//! Skinned characters: model loading (`resources/character/*_set`), animation clips bound
//! to a skeleton, CPU skinning into a per-frame vertex buffer. Shared by the pedestrians
//! and the drivers sitting in the cars.

use crate::assets::load_resource_file;
use crate::render::{Gpu, MatMode, Vertex};
use glam::{Mat4, Quat, Vec3};
use mcla_formats::rage::anim::{Animation, KIND_MOVER_ROTATION, KIND_MOVER_TRANSLATION, KIND_ROTATION, KIND_TRANSLATION};
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
        Some(SkinModel { bones, ids, lods, binds, head: c.bone("head") })
    }
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
        let parts = (lod.min(2)..3).map(|l| &model.lods[l]).find(|l| !l.is_empty()).unwrap_or(&model.lods[0]);
        // (the night fill light at the figure's place - fillmap.rs: the figures share one buffer
        // and the identity object, so the colour rides in every vertex: color1.rgb = four times
        // map / 255 x 0.25 x m_NFLFade, uv1.y = 77 the mark city.wgsl knows a figure by)
        let fill = crate::fillmap::figure_now(root.w_axis.x, root.w_axis.z);
        for part in parts {
            let base = self.verts.len() as u32;
            for v in &part.verts {
                let mut pos = Vec3::ZERO;
                let mut nrm = Vec3::ZERO;
                for k in 0..4 {
                    if v.w[k] > 0.0 {
                        let m = &self.skin[v.b[k] as usize];
                        pos += m.transform_point3(v.pos) * v.w[k];
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
        // (the lit figures' lanes: rows 0..11 of the object's first lamp table, the mark in row 0
        // of the second - city.wgsl, the walkers at night)
        for (i, l) in self.lanes.iter().enumerate() {
            let mut u = crate::render::ObjectUniform::default();
            u.lamp_a[..12].copy_from_slice(l);
            u.lamp_b[0][3] = 1.0;
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
