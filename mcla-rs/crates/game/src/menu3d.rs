//! The menus' 3D scene, as the original game builds it from the movie itself.
//!
//! The pause and garage movies do not go to the screen directly. Each one
//! describes a small scene in `_global` (`scenegraph`, `meshes`, `player0_cam`,
//! `lights`) and the game's Flash-3D pass (`F-13913`..`F-95656`) renders it:
//! the movie is the texture of a flat mesh (`useMovieAsTexture`), a glass frame
//! mesh (`pause_screen`) is stretched round the menu, and both hang in the world
//! next to the car. The movie's own `MoveMeshes` computes every transform each
//! frame from the menu's size and a per-view placement
//! (`cur_MenuPosition` / `cur_menuRotation` / `cur_MenuScale`).
//!
//! Read from the executable:
//! - a node's matrix (`F-12621`, row vectors): `S * Rx(rot0) * Rz(rot2)`,
//!   then the translation, then `Ry(rot1)`; a child is `local * parent`
//!   (`F-14472`), and a `world` view mesh is then taken through the matrix of
//!   its `coord` frame;
//! - a skinned mesh is not scaled as a whole: the bones' translations are
//!   multiplied by the node's scale (`A-22650`), so the frame's four corner
//!   bones move apart and its corners keep their shape;
//! - the pause cameras are `PauseCamera<n>Position/LookAt/MenuPos/MenuScale/
//!   MenuRot/Speed/FOVY` of `tune/ui` `UILogic` (`F-59198`), handed to the
//!   movie as `camera_coord[n - 1]` = position, look-at, menu position, rotation,
//!   speed, fov, scale x, scale y (`F-92151`); `FSCommand:SetPolarCam`
//!   (`F-32916`) blends two of them into `player0_cam`.
//!
//! The port draws through egui, so instead of a render target the movie's
//! triangles are carried through the same matrices vertex by vertex.

use egui::{Color32, Pos2, Rect, TextureHandle};
use glam::{Mat4, Vec3, Vec4};
use hashbrown::HashMap;
use mcla_flash::player::Player;
use mcla_flash::value::{Ref, Value};
use mcla_formats::rage::texture::Texture;
use std::path::Path;

/// One `PauseCamera<n>` of `UILogic`, in the car's frame (x right, y up, z rear).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PauseCam {
    pub position: Vec3,
    pub lookat: Vec3,
    pub menu_pos: Vec3,
    pub menu_scale: [f32; 2],
    pub menu_rot: f32,
    pub speed: f32,
    pub fovy: f32,
}

#[derive(Clone, Debug, Default)]
pub struct UiLogic {
    /// index = `PauseCamera` number - 1 (the game's own table, 50 slots)
    pub pause_cams: Vec<PauseCam>,
    /// RadarNear, RadarFalloff, RadarExponent (the tune loader F-59198; read by the police radar, F-61846)
    pub radar: [f32; 3],
}

impl UiLogic {
    /// The `UILogic` block is one of the hash-named files of `tune/ui`; it is
    /// recognised by its content.
    pub fn load(data_dir: &str) -> UiLogic {
        let dir = Path::new(data_dir).join("tune/ui");
        let Ok(rd) = std::fs::read_dir(&dir) else { return UiLogic::default() };
        for e in rd.flatten() {
            if e.metadata().map_or(true, |m| !m.is_file() || m.len() > 200_000) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(e.path()) else { continue };
            if text.contains("UILogic {") && text.contains("PauseCamera1Position") {
                return UiLogic::parse(&text);
            }
        }
        log::warn!("menu3d: no UILogic tune file under {}", dir.display());
        UiLogic::default()
    }

    pub fn parse(text: &str) -> UiLogic {
        let mut cams: Vec<PauseCam> = Vec::new();
        let mut radar = [0.0f32; 3];
        for line in text.lines() {
            let mut it = line.split_whitespace();
            let Some(key) = it.next() else { continue };
            if let Some(i) = ["RadarNear", "RadarFalloff", "RadarExponent"].iter().position(|k| *k == key) {
                radar[i] = it.next().and_then(|x| x.parse().ok()).unwrap_or(0.0);
                continue;
            }
            let Some(rest) = key.strip_prefix("PauseCamera") else { continue };
            let digits = rest.chars().take_while(|c| c.is_ascii_digit()).count();
            let Ok(n) = rest[..digits].parse::<usize>() else { continue };
            if n == 0 || n > 50 {
                continue;
            }
            let v: Vec<f32> = it.filter_map(|x| x.parse().ok()).collect();
            if cams.len() < n {
                cams.resize(n, PauseCam::default());
            }
            let c = &mut cams[n - 1];
            let v3 = |v: &[f32]| Vec3::new(v.first().copied().unwrap_or(0.0), v.get(1).copied().unwrap_or(0.0), v.get(2).copied().unwrap_or(0.0));
            match &rest[digits..] {
                "Position" => c.position = v3(&v),
                "LookAt" => c.lookat = v3(&v),
                "MenuPos" => c.menu_pos = v3(&v),
                "MenuScale" => c.menu_scale = [v.first().copied().unwrap_or(1.0), v.get(1).copied().unwrap_or(1.0)],
                "MenuRot" => c.menu_rot = v.first().copied().unwrap_or(0.0),
                "Speed" => c.speed = v.first().copied().unwrap_or(5.0),
                "FOVY" => c.fovy = v.first().copied().unwrap_or(35.0),
                _ => {}
            }
        }
        UiLogic { pause_cams: cams, radar }
    }

    /// Write the table into a movie the way `F-92151` does.
    pub fn write_camera_coord(&self, p: &mut Player) {
        if self.pause_cams.is_empty() {
            return;
        }
        let rows: Vec<Value> = self
            .pause_cams
            .iter()
            .map(|c| {
                let f = [c.position.x, c.position.y, c.position.z, c.lookat.x, c.lookat.y, c.lookat.z, c.menu_pos.x, c.menu_pos.y, c.menu_pos.z, c.menu_rot, c.speed, c.fovy, c.menu_scale[0], c.menu_scale[1]];
                let v = f.iter().map(|x| Value::Float(*x)).collect();
                p.new_array(v)
            })
            .collect();
        let arr = p.new_array(rows);
        p.set_global("camera_coord", arr);
    }

    /// `FSCommand:SetPolarCam` (`F-32916`): `fs_SrcCamIndex`, `fs_DstCamIndex`
    /// and `fs_LerpFactor` pick a place between two cameras, written to the
    /// camera object named by the argument.
    ///
    /// (approx.) The original's blend (`F-22967`) is a polar one about the
    /// car; here the eye turns about the blended look-at point at a blended
    /// distance, which is the same at both ends.
    pub fn set_polar_cam(&self, p: &mut Player, target: &str) {
        let idx = |p: &mut Player, n: &str| p.get_global(n).map_or(0, |v| v.to_i32()).max(0) as usize;
        let (src, dst) = (idx(p, "fs_SrcCamIndex"), idx(p, "fs_DstCamIndex"));
        let t = p.get_global("fs_LerpFactor").map_or(1.0, |v| v.to_f32()).clamp(0.0, 1.0);
        let Some(b) = self.pause_cams.get(dst).copied() else { return };
        let a = self.pause_cams.get(src).copied().unwrap_or(b);
        let look = a.lookat.lerp(b.lookat, t);
        let (da, db) = (a.position - a.lookat, b.position - b.lookat);
        let dist = da.length() + (db.length() - da.length()) * t;
        let dir = if da.length() > 1e-3 && db.length() > 1e-3 {
            let q = glam::Quat::from_rotation_arc(da.normalize(), db.normalize());
            glam::Quat::IDENTITY.slerp(q, t) * da.normalize()
        } else {
            db.normalize_or(Vec3::Z)
        };
        let pos = look + dir * dist;
        let fov = a.fovy + (b.fovy - a.fovy) * t;
        for (i, v) in [pos.x, pos.y, pos.z].into_iter().enumerate() {
            p.set_path(&format!("{target}.position.{i}"), Value::Float(v));
        }
        for (i, v) in [look.x, look.y, look.z].into_iter().enumerate() {
            p.set_path(&format!("{target}.lookat.{i}"), Value::Float(v));
        }
        p.set_path(&format!("{target}.fovy"), Value::Float(fov));
    }
}

/// A mesh of the movie's scene with its matrices resolved.
#[derive(Clone, Debug)]
pub struct Node {
    pub file: String,
    /// rotation and translation only (what a skinned mesh is drawn with)
    pub rt: Mat4,
    /// with the node's own scale
    pub srt: Mat4,
    pub scale: Vec3,
    /// 0..1
    pub alpha: f32,
    pub color: [u8; 3],
    pub movie: bool,
    /// `world` (the game's camera and the `coord` frame) or the name of a viewport
    pub view: String,
    pub coord: i32,
}

#[derive(Clone, Copy, Debug)]
pub struct Cam {
    pub position: Vec3,
    pub lookat: Vec3,
    /// degrees
    pub fovy: f32,
    pub coord: i32,
}

#[derive(Clone, Debug)]
pub struct Scene {
    /// the movie may show past its stage (the playlist box, wider than the stage)
    pub wide_stage: bool,
    pub nodes: Vec<Node>,
    pub cam: Cam,
    /// `lights[0]`: where the light shines to, its diffuse and ambient strength
    pub light_dir: Vec3,
    pub diffuse: f32,
    pub ambient: f32,
}

fn member(p: &mut Player, r: Ref, name: &str) -> Option<Value> {
    p.get_member(r, &Value::str(name))
}

fn vec3_of(p: &mut Player, r: Ref, name: &str, default: Vec3) -> Vec3 {
    let Some(a) = member(p, r, name).and_then(|v| v.obj()) else { return default };
    let mut out = default;
    for i in 0..3 {
        if let Some(v) = p.get_member(a, &Value::Int(i as i32)) {
            if !matches!(v, Value::Undef) {
                out[i] = v.to_f32();
            }
        }
    }
    out
}

/// `F-12621`: (rotation and translation, the same with the scale).
fn local(p: &mut Player, r: Ref) -> (Mat4, Vec3) {
    let rot = vec3_of(p, r, "rot", Vec3::ZERO);
    let scale = vec3_of(p, r, "scale", Vec3::ONE);
    let trans = vec3_of(p, r, "trans", Vec3::ZERO);
    let rt = Mat4::from_rotation_y(rot.y) * Mat4::from_translation(trans) * Mat4::from_rotation_z(rot.z) * Mat4::from_rotation_x(rot.x);
    (rt, scale)
}

fn items(p: &mut Player, r: Ref, name: &str) -> Vec<Ref> {
    let Some(a) = member(p, r, name).and_then(|v| v.obj()) else { return Vec::new() };
    let mut out = Vec::new();
    for i in 0..64 {
        match p.get_member(a, &Value::Int(i)).and_then(|v| v.obj()) {
            Some(o) => out.push(o),
            None => break,
        }
    }
    out
}

fn mesh_node(p: &mut Player, m: Ref, world: Mat4) -> Option<Node> {
    let file = member(p, m, "file").map(|v| v.to_string().to_string()).unwrap_or_default();
    if file.is_empty() {
        return None;
    }
    let (mrt, mscale) = local(p, m);
    let alpha = member(p, m, "_alpha").map_or(0.0, |v| v.to_f32()) * 0.01;
    let color = member(p, m, "color").map_or(0xffffff, |v| v.to_i32()) as u32;
    Some(Node {
        file,
        rt: world * mrt,
        srt: world * mrt * Mat4::from_scale(mscale),
        scale: mscale,
        alpha: alpha.clamp(0.0, 1.0),
        color: [(color >> 16) as u8, (color >> 8) as u8, color as u8],
        movie: member(p, m, "useMovieAsTexture").map_or(false, |v| v.to_i32() != 0),
        view: member(p, m, "view").map(|v| v.to_string().to_string()).unwrap_or_default(),
        coord: member(p, m, "coord").map_or(0, |v| v.to_i32()),
    })
}

fn walk(p: &mut Player, group: Ref, parent: Mat4, out: &mut Vec<Node>, depth: usize) {
    if depth > 8 {
        return;
    }
    let (rt, scale) = local(p, group);
    let world = parent * rt * Mat4::from_scale(scale);
    for m in items(p, group, "meshes") {
        out.extend(mesh_node(p, m, world));
    }
    for c in items(p, group, "children") {
        walk(p, c, world, out, depth + 1);
    }
}

/// The movie's scene as it stands after this frame's scripts.
pub fn read_scene(p: &mut Player) -> Option<Scene> {
    let mut nodes = Vec::new();
    let groups = {
        let mut v = Vec::new();
        for i in 0..16 {
            match p.get_path(&format!("scenegraph.{i}")).and_then(|v| v.obj()) {
                Some(o) => v.push(o),
                None => break,
            }
        }
        v
    };
    for g in groups {
        walk(p, g, Mat4::IDENTITY, &mut nodes, 0);
    }
    // meshes outside the graph (`dyn3Ds`: the phone's body, lid and screen)
    for i in 0..16 {
        match p.get_path(&format!("dyn3Ds.{i}")).and_then(|v| v.obj()) {
            Some(m) => nodes.extend(mesh_node(p, m, Mat4::IDENTITY)),
            None => break,
        }
    }
    if nodes.is_empty() {
        return None;
    }
    let (mut light_dir, mut diffuse, mut ambient) = (Vec3::new(-0.5, -0.5, -1.0), 1.0, 0.0);
    if let Some(l) = p.get_path("lights.0").and_then(|v| v.obj()) {
        light_dir = vec3_of(p, l, "direction", light_dir);
        diffuse = vec3_of(p, l, "diffuse", Vec3::ONE).x;
        ambient = vec3_of(p, l, "ambient", Vec3::ZERO).x;
    }
    let cam = p.get_path("player0_cam")?.obj()?;
    let cam = Cam {
        position: vec3_of(p, cam, "position", Vec3::new(0.0, 0.0, 2.5)),
        lookat: vec3_of(p, cam, "lookat", Vec3::ZERO),
        fovy: member(p, cam, "fovy").map_or(70.0, |v| v.to_f32()),
        coord: member(p, cam, "coord").map_or(0, |v| v.to_i32()),
    };
    Some(Scene { nodes, cam, light_dir, diffuse, ambient, wide_stage: false })
}

/// A UI mesh (`resources/ui/meshes/<name>.xdr`).
pub struct UiMesh {
    pub pos: Vec<Vec3>,
    pub normal: Vec<Vec3>,
    pub uv: Vec<[f32; 2]>,
    /// bind position of the bone each vertex follows (zero when unskinned)
    pub bone: Vec<Vec3>,
    pub indices: Vec<u32>,
    pub texture: String,
    pub skinned: bool,
    pub min: Vec3,
    pub max: Vec3,
}

impl UiMesh {
    pub fn load(data_dir: &str, name: &str) -> Option<UiMesh> {
        let dir = Path::new(data_dir).join("resources/ui/meshes");
        let plain = dir.join(format!("{name}.r102"));
        let hashed = dir.join(format!("0x{:08x}.r102", mcla_formats::hash::joaat(&format!("{name}.xdr"))));
        let path = if plain.exists() { plain } else { hashed };
        let res = crate::assets::load_resource_file(&path).ok()?;
        let d = mcla_formats::rage::scan::Drawable::parse(&res).ok()?;
        let rd = mcla_formats::rage::Rd::new(&res);
        // skeleton data at root+0x0c: +0x04 parents, +0x10 local matrices, +0x14 count
        let mut bones: Vec<Vec3> = Vec::new();
        if let Some(sk) = rd.ptr(0x0c) {
            let n = rd.u16(sk + 0x14) as usize;
            if let (Some(mats), true) = (rd.ptr(sk + 0x10), n > 0 && n <= 64) {
                if rd.has(mats, n * 64) {
                    let parents = rd.ptr(sk + 0x04).filter(|pa| rd.has(*pa, n * 4));
                    let t: Vec<Vec3> = (0..n).map(|i| Vec3::new(rd.f32(mats + i * 64 + 48), rd.f32(mats + i * 64 + 52), rd.f32(mats + i * 64 + 56))).collect();
                    for i in 0..n {
                        // the bind pose is translations only: sum them up the chain
                        let (mut w, mut cur, mut guard) = (t[i], i, 0);
                        while let Some(pa) = parents {
                            let par = rd.u32(pa + cur * 4) as usize;
                            if par >= n || par == cur || guard > 16 {
                                break;
                            }
                            w += t[par];
                            cur = par;
                            guard += 1;
                        }
                        bones.push(w);
                    }
                }
            }
        }
        let mut m = UiMesh { pos: Vec::new(), normal: Vec::new(), uv: Vec::new(), bone: Vec::new(), indices: Vec::new(), texture: String::new(), skinned: false, min: Vec3::splat(f32::MAX), max: Vec3::splat(f32::MIN) };
        for model in &d.models {
            for (mesh, si) in &model.parts {
                let base = m.pos.len() as u32;
                if m.texture.is_empty() {
                    m.texture = d.shader_group.shaders.get(*si as usize).and_then(|s| s.diffuse()).unwrap_or("").to_ascii_lowercase();
                }
                for i in 0..mesh.positions.len() {
                    let p = mesh.positions[i];
                    m.min = m.min.min(p);
                    m.max = m.max.max(p);
                    m.pos.push(p);
                    m.normal.push(mesh.normals.get(i).copied().unwrap_or(Vec3::Y));
                    m.uv.push(mesh.uv0.get(i).map_or([0.0, 0.0], |u| [u.x, u.y]));
                    let mut b = Vec3::ZERO;
                    if let (Some(bi), Some(bw), false) = (mesh.blend_indices.get(i), mesh.blend_weights.get(i), bones.is_empty()) {
                        let w = bw.to_array();
                        let slot = (0..4).max_by(|a, c| w[*a].total_cmp(&w[*c])).unwrap_or(0);
                        let idx = mesh.bone_palette.get(bi[slot] as usize).map_or(bi[slot] as usize, |x| *x as usize);
                        if w[slot] > 0.0 {
                            if let Some(t) = bones.get(idx) {
                                b = *t;
                                m.skinned = true;
                            }
                        }
                    }
                    m.bone.push(b);
                }
                m.indices.extend(mesh.indices.iter().map(|i| base + *i));
            }
        }
        if m.pos.is_empty() { None } else { Some(m) }
    }
}

/// The meshes and their textures (`resources/ui/meshtextures.r9`), loaded on first use.
pub struct Meshes {
    data_dir: String,
    meshes: HashMap<String, Option<UiMesh>>,
    dict: Option<HashMap<String, Texture>>,
    handles: HashMap<String, Option<TextureHandle>>,
}

impl Meshes {
    pub fn new(data_dir: &str) -> Meshes {
        Meshes { data_dir: data_dir.to_string(), meshes: HashMap::new(), dict: None, handles: HashMap::new() }
    }

    pub fn mesh(&mut self, name: &str) -> Option<&UiMesh> {
        let dir = self.data_dir.clone();
        self.meshes
            .entry(name.to_string())
            .or_insert_with(|| {
                let m = UiMesh::load(&dir, name);
                if m.is_none() {
                    log::warn!("menu3d: mesh {} not found", name);
                }
                m
            })
            .as_ref()
    }

    fn texture(&mut self, ctx: &egui::Context, name: &str) -> Option<TextureHandle> {
        if let Some(h) = self.handles.get(name) {
            return h.clone();
        }
        let dict = self.dict.get_or_insert_with(|| {
            use mcla_formats::rage::{Dictionary, Rd};
            let mut out = HashMap::new();
            if let Ok(res) = crate::assets::load_resource_file(&Path::new(&self.data_dir).join("resources/ui/meshtextures.r9")) {
                let rd = Rd::new(&res);
                if let Ok(d) = Dictionary::parse(&rd, 0) {
                    for o in d.values.iter().flatten() {
                        if let Ok(t) = Texture::parse(&rd, *o) {
                            out.insert(t.name.to_ascii_lowercase(), t);
                        }
                    }
                }
            }
            out
        });
        // dictionary names carry the mesh's folder ("pause_screen/pausebg_ps3.dds")
        let h = dict.iter().find(|(k, _)| k.rsplit(['/', '\\']).next() == Some(name)).map(|(_, t)| {
            let img = crate::flash_ui::plain_image(t.width, t.height, &t.to_rgba8());
            ctx.load_texture(format!("uimesh:{name}"), img, egui::TextureOptions::LINEAR_REPEAT)
        });
        self.handles.insert(name.to_string(), h.clone());
        h
    }
}

/// The glass frame (`pause_screen`): its texture is a 60% white pane with a full
/// white line near the rim, and its inner sheet repeats the pane across the whole
/// panel. The original draws the movie's surface in front of it with depth, so the
/// pane only shows where the movie is clear - the margin between the rim line and
/// the dark panels. Here the pane lies under the movie, which is drawn this much
/// denser so that its dark panels come out as dark as on bare ground (for a dark
/// scene: 0.77 becomes 0.91).
const MOVIE_OVER_GLASS: f32 = 1.18;

/// The frame's strength against the mesh's own alpha (50%) times its texture's (60%
/// pane, full rim line). Measured on three references (pause `q20`, garage `t30`, the
/// flat garage pages `t44`), as the share of white over the scene behind: rim line
/// 0.47-0.53, pane 0.20-0.27. Drawn as it is the port came out at 0.62-0.75 and
/// 0.30-0.33 - without depth the frame's bevel faces lie over its top faces and blend
/// twice - so the whole frame is taken down by the ratio (approx.).
const FRAME_ALPHA: f32 = 0.72;

/// Brightness of a lit UI mesh (see `paint`).
const FRAME_LIGHT: f32 = 1.0;

/// Stage pixels to the mesh's own space, from the mesh's texture coordinates: the
/// movie is the mesh's texture, u and v running 0..1 over the stage.
fn stage_on_mesh(mesh: &UiMesh, stage: [f32; 4]) -> Option<Mat4> {
    let (w, h) = ((stage[1] - stage[0]).max(1.0), (stage[3] - stage[2]).max(1.0));
    for t in mesh.indices.chunks_exact(3) {
        let (i0, i1, i2) = (t[0] as usize, t[1] as usize, t[2] as usize);
        let (p0, p1, p2) = (mesh.pos[i0], mesh.pos[i1], mesh.pos[i2]);
        let (a0, a1, a2) = (mesh.uv[i0], mesh.uv[i1], mesh.uv[i2]);
        let (du1, dv1, du2, dv2) = (a1[0] - a0[0], a1[1] - a0[1], a2[0] - a0[0], a2[1] - a0[1]);
        let det = du1 * dv2 - du2 * dv1;
        if det.abs() < 1e-6 {
            continue;
        }
        let (e1, e2) = (p1 - p0, p2 - p0);
        // position = origin + u * su + v * sv
        let su = (e1 * dv2 - e2 * dv1) / det;
        let sv = (e2 * du1 - e1 * du2) / det;
        let origin = p0 - su * a0[0] - sv * a0[1];
        let (cx, cy) = (su / w, sv / h);
        return Some(Mat4::from_cols(cx.extend(0.0), cy.extend(0.0), su.cross(sv).normalize_or(Vec3::Y).extend(0.0), (origin - cx * stage[0] - cy * stage[2]).extend(1.0)));
    }
    None
}

/// How a view of the scene reaches the screen.
#[derive(Clone, Copy)]
pub struct View {
    /// projection * view
    pub vp: Mat4,
    /// the `coord` frame's matrix (identity when the camera is given in that frame)
    pub frame: Mat4,
}

impl View {
    /// A camera given by eye, target and vertical field of view (degrees).
    pub fn look(position: Vec3, lookat: Vec3, fovy: f32, aspect: f32) -> View {
        let v = Mat4::look_at_rh(position, lookat, Vec3::Y);
        let p = Mat4::perspective_rh(fovy.clamp(0.5, 170.0).to_radians(), aspect.max(0.1), 0.05, 2000.0);
        View { vp: p * v, frame: Mat4::IDENTITY }
    }
}

/// Clip space to screen; `None` behind the eye.
pub fn to_screen(clip: Vec4, screen: Rect) -> Option<Pos2> {
    if clip.w <= 1e-3 {
        return None;
    }
    let (x, y) = (clip.x / clip.w, clip.y / clip.w);
    Some(Pos2::new(screen.center().x + x * screen.width() * 0.5, screen.center().y - y * screen.height() * 0.5))
}

/// Draw the scene: every mesh in the order the movie lists it, the movie itself
/// on the mesh that takes it as its texture.
///
/// `world` is the game's camera and frame for `world` view meshes; without one
/// the movie's own camera is used (camera and meshes then share the frame, so
/// the picture is the same). Viewport meshes always use the movie's camera.
pub fn paint(ctx: &egui::Context, painter: &egui::Painter, meshes: &mut Meshes, movie: &mut crate::flash_ui::FlashMovie, scene: &Scene, world: Option<View>) {
    paint_scene(ctx, painter, meshes, movie, scene, world, false)
}

/// `backdrop`: draw the scene's black backdrop mesh (`pause_blackBG`) as a black
/// screen behind the panel - the full-screen templates (vehicle list, showroom)
/// stand on it. It is a wall far behind the menu that the original sorts by depth;
/// without depth it can only be all or nothing here.
pub fn paint_scene(ctx: &egui::Context, painter: &egui::Painter, meshes: &mut Meshes, movie: &mut crate::flash_ui::FlashMovie, scene: &Scene, world: Option<View>, backdrop: bool) {
    let screen = ctx.screen_rect();
    if backdrop {
        if let Some(n) = scene.nodes.iter().find(|n| n.file.contains("black") && n.alpha > 0.004) {
            painter.rect_filled(screen, 0.0, crate::flash_ui::plain_color(0, 0, 0, (n.alpha * 255.0) as u8));
        }
    }
    let own = View::look(scene.cam.position, scene.cam.lookat, scene.cam.fovy, screen.width() / screen.height().max(1.0));
    let list = mcla_flash::draw::build(&movie.player);
    // nearer surfaces last: the black backdrop, the glass frame, then the movie
    let rank = |n: &Node| if n.movie { 2 } else if n.file.contains("black") { 0 } else { 1 };
    if std::env::var("MCLA_MENU3D_LOG").is_ok() {
        log::info!("menu3d: paint in pass {} ({} nodes)", ctx.cumulative_pass_nr(), scene.nodes.len());
        for n in &scene.nodes {
            log::info!("menu3d: {} view {} coord {} alpha {:.2} movie {} scale {:?}", n.file, n.view, n.coord, n.alpha, n.movie, n.scale);
        }
    }
    let glass = scene.nodes.iter().any(|n| n.file == "pause_screen" && n.alpha > 0.004);
    let mut order: Vec<&Node> = scene.nodes.iter().filter(|n| n.alpha > 0.004).collect();
    order.sort_by_key(|n| rank(n));
    for n in order {
        let view = if n.view == "world" { world.unwrap_or(own) } else { own };
        // (approx.) The black backdrop mesh (`pause_blackBG`, switched on by the 2D
        // templates) is a wall 312 units from the viewport's eye; the original draws it
        // with depth, so the car and the garage in front of it stay in view. Without a
        // depth buffer here it would black the scene out, so it is left out.
        if n.file.contains("black") {
            continue;
        }
        if std::env::var("MCLA_MENU3D_ONLY").map_or(false, |v| (v == "movie") != n.movie) {
            continue;
        }
        let Some(mesh) = meshes.mesh(&n.file) else { continue };
        if n.movie {
            // the stage lies on the mesh by its texture coordinates (u, v = stage x, y over
            // its width and height)
            let s = movie.player.movie.stage;
            let Some(stage_to_local) = stage_on_mesh(mesh, s) else { continue };
            let m = view.vp * view.frame * n.srt * stage_to_local;
            // (the movie is only drawn denser where a glass pane lies under it)
            let dense = if glass { MOVIE_OVER_GLASS } else { 1.0 };
            // the mesh shows the part of the stage its texture coordinates span (the phone's
            // screen is the upper part of a square stage)
            let (mut u0, mut v0, mut u1, mut v1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
            for uv in &mesh.uv {
                u0 = u0.min(uv[0]);
                v0 = v0.min(uv[1]);
                u1 = u1.max(uv[0]);
                v1 = v1.max(uv[1]);
            }
            let (w, h) = (s[1] - s[0], s[3] - s[2]);
            // (a mesh that takes the whole stage does not cut it: the playlist box is wider
            // than the stage and the original shows all of it)
            let whole = scene.wide_stage && u0 < 0.01 && v0 < 0.01 && u1 > 0.99 && v1 > 0.99;
            let part = if whole { [s[0] - 4.0 * w, s[2] - 4.0 * h, s[0] + 5.0 * w, s[2] + 5.0 * h] } else { [s[0] + u0 * w, s[2] + v0 * h, s[0] + u1 * w, s[2] + v1 * h] };
            if whole || !crate::selfmovie::enabled() {
                movie.paint_warped(ctx, painter, &list, m, screen, (n.alpha * dense).min(1.0), Some(part));
                continue;
            }
            // the original's way (`selfmovie`): the stage drawn flat into its 960 x 640 picture, the mesh
            // textured with it by its own texture coordinates
            let mm = view.vp * view.frame * n.srt;
            let tris: Vec<(Vec4, [f32; 2])> = mesh.indices.iter().map(|&i| (mm * mesh.pos[i as usize].extend(1.0), mesh.uv.get(i as usize).copied().unwrap_or([0.0; 2]))).collect();
            crate::selfmovie::paint_mesh(ctx, painter, screen, movie, &list, &tris, (n.alpha * dense).min(1.0));
            continue;
        }
        let tex_name = mesh.texture.clone();
        let skinned = mesh.skinned;
        if !skinned {
            // a solid object (the phone): lit by the scene's light, far faces first
            let model = view.frame * n.srt;
            let m = view.vp * model;
            let to_light = -(scene.light_dir.normalize_or(Vec3::NEG_Z));
            let clip: Vec<Vec4> = mesh.pos.iter().map(|p| m * p.extend(1.0)).collect();
            let shade: Vec<f32> = mesh.normal.iter().map(|nm| (scene.ambient + scene.diffuse * model.transform_vector3(*nm).normalize_or(Vec3::Y).dot(to_light).max(0.0)).clamp(0.0, 1.0)).collect();
            let (uv, idx) = (mesh.uv.clone(), mesh.indices.clone());
            let handle = meshes.texture(ctx, &tex_name);
            let mut tris: Vec<(f32, [u32; 3])> = Vec::with_capacity(idx.len() / 3);
            for t in idx.chunks_exact(3) {
                let (a, b, c) = (clip[t[0] as usize], clip[t[1] as usize], clip[t[2] as usize]);
                if a.w <= 1e-3 || b.w <= 1e-3 || c.w <= 1e-3 {
                    continue;
                }
                tris.push(((a.w + b.w + c.w) / 3.0, [t[0], t[1], t[2]]));
            }
            tris.sort_by(|x, y| y.0.total_cmp(&x.0));
            let mut out = egui::Mesh::default();
            if let Some(h) = &handle {
                out.texture_id = h.id();
            }
            let alpha = (n.alpha * 255.0) as u8;
            for (_, t) in tris {
                let base = out.vertices.len() as u32;
                for k in t {
                    let Some(pos) = to_screen(clip[k as usize], screen) else { continue };
                    let l = shade[k as usize];
                    let ch = |c: u8| (c as f32 * l) as u8;
                    let u = uv[k as usize];
                    out.vertices.push(egui::epaint::Vertex { pos, uv: if handle.is_some() { Pos2::new(u[0], u[1]) } else { egui::epaint::WHITE_UV }, color: crate::flash_ui::plain_color(ch(n.color[0]), ch(n.color[1]), ch(n.color[2]), alpha) });
                }
                if out.vertices.len() as u32 == base + 3 {
                    out.indices.extend([base, base + 1, base + 2]);
                } else {
                    out.vertices.truncate(base as usize);
                }
            }
            if !out.indices.is_empty() {
                painter.add(egui::Shape::mesh(out));
            }
            continue;
        }
        let m = view.vp * view.frame * if skinned { n.rt } else { n.srt };
        let mut out = egui::Mesh::default();
        let mut pts: Vec<Option<Pos2>> = Vec::with_capacity(mesh.pos.len());
        // which vertices face the eye: a step along the normal comes nearer
        let mut facing: Vec<bool> = Vec::with_capacity(mesh.pos.len());
        for i in 0..mesh.pos.len() {
            let p = if skinned { mesh.pos[i] + mesh.bone[i] * (n.scale - Vec3::ONE) } else { mesh.pos[i] };
            let c = m * p.extend(1.0);
            facing.push((m * (p + mesh.normal[i] * 0.01).extend(1.0)).w < c.w);
            pts.push(to_screen(c, screen));
        }
        let a = (n.alpha * FRAME_ALPHA * 255.0) as u8;
        // (the reference's frame line reads as white at the mesh's alpha: no shading applied)
        let lit = |c: u8| (c as f32 * FRAME_LIGHT) as u8;
        let color = crate::flash_ui::plain_color(lit(n.color[0]), lit(n.color[1]), lit(n.color[2]), a);
        let (uv, idx): (Vec<[f32; 2]>, Vec<u32>) = (mesh.uv.clone(), mesh.indices.clone());
        let mesh_normals = mesh.normal.clone();
        let handle = meshes.texture(ctx, &tex_name);
        if let Some(h) = &handle {
            out.texture_id = h.id();
        }
        for t in idx.chunks_exact(3) {
            let (Some(a0), Some(b0), Some(c0)) = (pts[t[0] as usize], pts[t[1] as usize], pts[t[2] as usize]) else { continue };
            // back faces are not drawn (the frame's rear plate faces away)
            if !(facing[t[0] as usize] && facing[t[1] as usize] && facing[t[2] as usize]) {
                continue;
            }
            let base = out.vertices.len() as u32;
            for (k, p) in [(t[0], a0), (t[1], b0), (t[2], c0)] {
                let u = uv[k as usize];
                out.vertices.push(egui::epaint::Vertex { pos: p, uv: if handle.is_some() { Pos2::new(u[0], u[1]) } else { egui::epaint::WHITE_UV }, color });
            }
            out.indices.extend([base, base + 1, base + 2]);
        }
        if std::env::var("MCLA_MENU3D_LOG").is_ok() {
            let (mut up, mut down, mut side) = (0, 0, 0);
            for t in idx.chunks_exact(3) {
                if facing[t[0] as usize] && facing[t[1] as usize] && facing[t[2] as usize] {
                    let ny = mesh_normals[t[0] as usize].y;
                    if ny > 0.9 { up += 1 } else if ny < -0.9 { down += 1 } else { side += 1 }
                }
            }
            log::info!("menu3d: {} drew {} of {} triangles (up {} down {} side {}), alpha {} color {:?} tex {:?}", n.file, out.indices.len() / 3, idx.len() / 3, up, down, side, a, color, handle.as_ref().map(|h| h.size()));
        }
        if !out.indices.is_empty() {
            painter.add(egui::Shape::mesh(out));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The radar's three values of the disc's UILogic block (`tune/ui/0x1020673f`)
    #[test]
    fn radar_tune_values() {
        assert_eq!(UiLogic::parse("UILogic {\n  RadarNear 40 \n  RadarFalloff 140 \n  RadarExponent 1 \n}\n").radar, [40.0, 140.0, 1.0]);
    }

    const TUNE: &str = "type: a\nUILogic {\n  RadarNear 40 \n  PauseCamera1Position -0.22499999\t0.94999999\t2.575001 \n  PauseCamera2Position -2.2750001\t-0.34999999\t3.5999999 \n  PauseCamera2LookAt -0.22499999\t-0.25\t-0.97500002 \n  PauseCamera2MenuPos -2\t-0.30000001\t0.050000001 \n  PauseCamera2MenuScale 1.6\t1.6 \n  PauseCamera2MenuRot 0 \n  PauseCamera2Speed 5 \n  PauseCamera2FOVY 53.75 \n}\n";

    #[test]
    fn pause_cameras_are_indexed_from_one_like_the_games_table() {
        let u = UiLogic::parse(TUNE);
        assert_eq!(u.pause_cams.len(), 2);
        assert_eq!(u.pause_cams[0].position, Vec3::new(-0.22499999, 0.94999999, 2.575001));
        let c = u.pause_cams[1];
        assert_eq!(c.menu_pos, Vec3::new(-2.0, -0.30000001, 0.050000001));
        assert_eq!((c.menu_scale, c.fovy, c.speed), ([1.6, 1.6], 53.75, 5.0));
    }

    /// The pause panel of the reference capture (static recomp, free roam, R8):
    /// with PauseCamera2 and a 480x347 menu the movie surface's corners project
    /// to the panel seen there (frame x 193..545, y 268..532 at 1280x720).
    #[test]
    fn the_pause_panel_lands_where_the_original_shows_it() {
        let c = UiLogic::parse(TUNE).pause_cams[1];
        let (w, h, uvx, uvy) = (480.0f32, 347.0f32, 508.0f32, 512.0f32);
        let (sx, sy) = (c.menu_scale[0], c.menu_scale[1]);
        // MoveMeshes, world branch
        let trans = Vec3::new(c.menu_pos.x - (w / uvx) * sx / 2.0 + 0.5 * sx, c.menu_pos.y + (h / uvy) * sy - 0.7 - 0.5 * sy, c.menu_pos.z);
        let group = Mat4::from_rotation_y(c.menu_rot) * Mat4::from_translation(trans) * Mat4::from_rotation_x(1.57) * Mat4::from_scale(Vec3::new(sx, sy, sy));
        let view = View::look(c.position, c.lookat, c.fovy, 1280.0 / 720.0);
        let screen = Rect::from_min_size(Pos2::ZERO, egui::vec2(1280.0, 720.0));
        let at = |x: f32, y: f32| to_screen(view.vp * group * Vec4::new(-0.5 + x / 512.0, 0.0, -0.5 + y / 512.0, 1.0), screen).unwrap();
        let (tl, br) = (at(0.0, 0.0), at(w, h));
        assert!((tl.x - 200.0).abs() < 6.0 && (tl.y - 273.0).abs() < 6.0, "top left {tl:?}");
        assert!((br.x - 538.0).abs() < 6.0 && (br.y - 501.0).abs() < 6.0, "bottom right {br:?}");
    }
}
