//! The original's rear-view mirror, which the shipped game never shows (OPTIONS > DISPLAY > REAR-VIEW MIRROR,
//! the port's own switch for it - DECISIONS.md D20; rules and measurements in docs/LIGHTING.md "Rear-view
//! mirror (2026-10-21)").
//!
//! In the original the HUD's `rearView` object (class vtable 0x82027A54, made by F-21135 from the HUD
//! builder F-97322) runs only when the RAGE command-line parameter `rearview` is set (0x8287E0A8 +4,
//! registered by F-82889). Each frame (F-16951) it turns render pass 0x01000000 on while the HUD
//! element is up (+16 bit 0x20 - off in the pause menu) and off otherwise - at once, no fade: the fade at +1216
//! (4 / s, 0x8200308C) only sets how strongly the overlay frame textures rear_view1 / rear_view_2 / _1bg / _2bg
//! are drawn (F-64327 -> F-46997), and the shipped data has none of them, so nothing of it shows.
//!
//! The pass (F-27689 0x01000000 -> F-40143, its viewport object *(0x8288D004) made by F-95690):
//! - the rectangle (960, 57, 256, 51) of the 1280 x 720 frame (+0 .. +12), the window set by F-99389;
//! - a perspective of 15 degrees vertical (+20, 0x82007F9C) at the aspect of that rectangle, 256 / 51,
//!   with the main view's near and far (main viewport +704 / +708: 0.2, 6000 recorded) - F-79482;
//! - the camera: the player's car matrix (F-35658 of F-55479(*(0x82874374))) - its body's roll and
//!   pitch with it - at the car's origin + 1.5 m straight up (world y, 0x827E5804), its first and third rows
//!   times -1 (0x82003770): turned half round about the car's up axis, looking backwards, NOT mirrored left to
//!   right. (Without a player car: the main view's camera.)
//! - drawn into the scene's own target and depth after the main view's opaque and glass draws, before its
//!   shadow-collect, glow, fog and particle passes: the strip goes through the same post chain.
//! What it draws (GPU recordings `work/lighting2/rear/cap/rv1`, `st1`, `nt1`): the city's sector and far
//! models with the main view's programs (day and night ones), the ocean water batches, the traffic cars
//! (VSAmbient*), and the sky dome with vs_main + ps_main_cheap; no props, foliage, pedestrians, ground dome,
//! glows, particles, player car.
//!
//! The traffic's LOD in the pass (F-27833 0x01000000 -> F-68463 -> F-69447, flag 1): the
//! mirror's view; a fixed sphere radius 0.9 (0x82007FA8); the steps x 0.125 (0x82003088); -1 when deeper than
//! 125 (0x820375A8); LOD 0 drawn as LOD 1 - `mirror_lod`.

use crate::render::{Frustum, Globals, Gpu, MatMode};
use crate::vehicles::Vehicles;
use glam::{Mat4, Quat, Vec3};

/// The mirror's rectangle in the original's 1280 x 720 frame: x, y, width, height (F-95690).
pub const RECT: [f32; 4] = [960.0, 57.0, 256.0, 51.0];
/// Vertical field of view, degrees (+20 = 0x82007F9C).
pub const FOV_Y_DEG: f32 = 15.0;
/// The eye above the car's origin, metres, along the world's y (0x827E5804).
pub const EYE_UP: f32 = 1.5;
/// The traffic's LOD sphere in the mirror (0x82007FA8), its step scale (0x82003088) and depth limit (0x820375A8).
pub const LOD_RADIUS: f32 = 0.9;
pub const LOD_STEP_SCALE: f32 = 0.125;
pub const LOD_MAX_DEPTH: f32 = 125.0;
/// The sectors drawn in detail in the mirror: loaded and nearer than this to the main camera (0x827E0E54).
pub const MIRROR_DETAIL_DIST: f32 = 20.0;

/// The mirror's rectangle on a `w` x `h` target: the original's fixed rectangle of its 1280 x 720 frame, the
/// frame scaled to the target as a whole (x by w / 1280, y by h / 720).
pub fn rect(w: u32, h: u32) -> [f32; 4] {
    let (sx, sy) = (w as f32 / 1280.0, h as f32 / 720.0);
    [RECT[0] * sx, RECT[1] * sy, RECT[2] * sx, RECT[3] * sy]
}

/// The mirror's eye and view from the car's pose: the car's rotation turned half round about its own up axis
/// (rows a and c times -1), the eye 1.5 m above the car's origin.
pub fn camera(car_pos: Vec3, car_rot: Quat) -> (Vec3, Mat4) {
    let eye = car_pos + Vec3::Y * EYE_UP;
    let rot = car_rot * Quat::from_rotation_y(std::f32::consts::PI);
    (eye, Mat4::from_rotation_translation(rot, eye).inverse())
}

/// The mirror's projection, the port's reverse-Z form (camera.rs `Camera::proj`) at 15 degrees and the
/// rectangle's own aspect, 256 / 51, with the main view's near.
pub fn proj(near: f32) -> Mat4 {
    let f = 1.0 / (FOV_Y_DEG.to_radians() * 0.5).tan();
    let aspect = RECT[2] / RECT[3];
    Mat4::from_cols_array(&[f / aspect, 0.0, 0.0, 0.0, 0.0, f, 0.0, 0.0, 0.0, 0.0, 0.0, -1.0, 0.0, 0.0, near, 0.0])
}

/// F-69447 with flag 1, then the caller's LOD 0 -> 1: the LOD of a traffic car at depth `d` (the
/// sphere's centre along the mirror's axis, less the near distance, plus 0.9) - `steps` the type's two depths
/// of the main view (`CarModel::lod_steps`), here x 0.125. -1: not drawn.
pub fn mirror_lod(d: f32, steps: [f32; 2]) -> i8 {
    if d > LOD_MAX_DEPTH {
        -1
    } else if d < steps[1] * LOD_STEP_SCALE {
        // (below the first step it is LOD 0, which the mirror's pass draws as LOD 1)
        1
    } else {
        2
    }
}

/// Every car's LOD in the mirror (`Vehicles::lod_pick`'s layout): traffic cars by `mirror_lod` when the
/// sphere is in the mirror's view and not behind the main view's occluders (F-87100 is asked as in the
/// main view's pass); cars without LODs - the player's, racers, set-up cars - are not the traffic's pass and
/// are left out (-1).
pub fn pick_lods(v: &Vehicles, eye: Vec3, axis: Vec3, near: f32, frustum: &Frustum, occluders: &crate::occluder::Selected) -> Vec<i8> {
    v.cars
        .iter()
        .map(|c| {
            if c.model.lods[0].0.is_empty() || c.is_player || c.pu_stealth {
                return -1;
            }
            let pose = c.render_pose(v.render_alpha);
            let centre = pose.pos + pose.rot * c.model.lod_centre;
            if !frustum.intersects_sphere(centre, LOD_RADIUS) || occluders.hides_sphere(centre, LOD_RADIUS) {
                return -1;
            }
            let d = (centre - eye).dot(axis) - near + LOD_RADIUS;
            let l = mirror_lod(d, c.model.lod_steps);
            static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            if l >= 0 && std::env::var("MCLA_REARVIEW_LOG").is_ok() && N.fetch_add(1, std::sync::atomic::Ordering::Relaxed) % 30 == 0 {
                eprintln!("rearview car {} lod {} d {:.1}", c.model.name, l, d);
            }
            l
        })
        .collect()
}

/// The ground-plane gap from `p` to a sector's box (F-82919: x and z only, 0 over the box) - the record's +268: the
/// LARGER of the two plan gaps, not their hypotenuse (read live off the original's 109 sector records, 109 of 109
/// within 0.05 m, LIGHTING.md "Env far models decoded"). MCLA_REARVIEW_DIST_OLD=1: the hypotenuse, as until then.
pub fn sector_distance(min: [f32; 3], max: [f32; 3], p: Vec3) -> f32 {
    let c = p.clamp(Vec3::from(min), Vec3::from(max));
    let (gx, gz) = ((c.x - p.x).abs(), (c.z - p.z).abs());
    if std::env::var_os("MCLA_REARVIEW_DIST_OLD").is_some() { (gx * gx + gz * gz).sqrt() } else { gx.max(gz) }
}

/// Whether the sector drawable of this bucket (the low three bits of its shader word, F-21213) is drawn in the
/// mirror: F-65459 draws buckets 0, 2 and 4 for pass 0x01000000 and bucket 1 (the blended windows and signs) only
/// for passes with a bit of 0x1F0; bucket 3 (the sea) comes from the water's own path, as in the main view.
pub fn mirror_bucket(flags: u32) -> bool {
    flags & 7 != 1
}

/// F-75080 in the mirror's pass (0x01000000): a sector's detailed model only when it is loaded (state 3) and
/// its +268 - the larger plan gap from the LOD view (the main camera + 10 m back) to its box - is under 20 m (0x827E0E54, never
/// written); else its far model (when the far city is there, 0x8287C7D4). No cross-fade in this pass (its alpha
/// is set only for the passes of bits 11 .. 20), so a far model is drawn whole even where the main view has
/// faded it out.
pub fn mirror_detail(loaded: bool, dist: f32) -> bool {
    loaded && dist < MIRROR_DETAIL_DIST
}

/// The models the mirror draws, each with whether its own object uniform (the cross-fade alpha, interiors) is
/// bound, and for a far model the sphere round its sector's box: the loaded sectors under 20 m (drawn drawable by
/// drawable), and the far model of every other sector (the plain uniform: whole). A far model is
/// drawn whole when that sphere is in the mirror's view (record +292 bit 0x400, F-56244 -> F-29884 on
/// the mirror's viewport): o14's 100 VSCityLOD / VSCityWindowLOD draws of the strip are exactly the far batches
/// of the port's sectors chosen so, by index count.
fn models(world: &crate::world::World, main_eye: Vec3) -> Vec<(&crate::world::GpuSector, bool, Option<(Vec3, f32)>)> {
    let near = |id: usize| world.index.sectors.get(id).map_or(false, |si| mirror_detail(world.loaded.contains_key(&id), sector_distance(si.min, si.max, main_eye)));
    let sphere = |id: usize| world.index.sectors.get(id).map(|si| {
        let (mn, mx) = (Vec3::from(si.min), Vec3::from(si.max));
        ((mn + mx) * 0.5, (mx - mn).length() * 0.5)
    });
    let mut out: Vec<(&crate::world::GpuSector, bool, Option<(Vec3, f32)>)> = world.loaded.iter().filter(|(id, _)| near(**id)).map(|(_, s)| (s, true, None)).collect();
    // (the garage rooms - the port's interior sets, `world::SET_ID` - are not in the mirror: the original's pass draws
    // the sectors' drawables and far models only, F-75080, and o14 has none of the rooms' index counts in any pass)
    out.extend(world.far_models_by_sector().filter(|(sid, _)| !near(*sid)).map(|(sid, s)| (s, false, sphere(sid))));
    out
}

/// The mirror's frame: its globals (the main view's with the mirror's camera) and the matrices.
pub struct View {
    pub eye: Vec3,
    /// the LOD view's eye: the sectors' distances (+268) are measured from it - the main camera moved back along its
    /// own back axis by the tunable 10 (F-66201: the viewport's camera, position + back axis x [10]; read live, 10.0000)
    pub main_eye: Vec3,
    pub axis: Vec3,
    pub frustum: Frustum,
    pub globals: Globals,
}

pub fn view(main: &Globals, car_pos: Vec3, car_rot: Quat, near: f32, scene: (u32, u32)) -> View {
    let (eye, view) = camera(car_pos, car_rot);
    // (drawn with pix_center 0: half a pixel of its rectangle on the scene - render::half_pixel)
    let r = rect(scene.0, scene.1);
    // (test hook MCLA_REARVIEW_VP=<16 numbers>: the recorded c8 - c11 as the view instead, the depth row the port's, to
    // measure the strip's offset without the car's pose in it)
    let base = std::env::var("MCLA_REARVIEW_VP").ok().map(|v| v.split(',').filter_map(|x| x.trim().parse::<f32>().ok()).collect::<Vec<f32>>()).filter(|v| v.len() == 16).map_or(proj(near) * view, |v| {
        let mut m = v;
        m[2] = 0.0;
        m[6] = 0.0;
        m[10] = 0.0;
        m[14] = near;
        Mat4::from_cols_array(&m.try_into().unwrap())
    });
    let vp = crate::render::half_pixel(base, r[2], r[3]);
    let mut g = *main;
    g.view_proj = vp.to_cols_array_2d();
    g.inv_view_proj = vp.inverse().to_cols_array_2d();
    g.cam_pos = [eye.x, eye.y, eye.z, main.cam_pos[3]];
    // (no sun shadows: the original's mirror pass binds an 8 x 8 stand-in as the city programs'
    // ShadowCollectorSampler (s13) in place of the frame's 1280 x 720 collector - recorded in o14, draw 3137
    // against 2189 - and its strip shows the street sunlit. MCLA_REARVIEW_SHADOWS=1: with the main view's.)
    if std::env::var("MCLA_REARVIEW_SHADOWS").is_err() {
        g.shadow_params[0] = 0.0;
    }
    // (the mirror's cut-outs: every draw of the pass has colour control 0x8700000c - the alpha test on, GREATER
    // than RB_ALPHA_REF 0.0, alpha-to-mask off - where the main view's cut-out draws have 0x8700001c (alpha-to-mask,
    // the port's half-alpha cut): o14 draws 3143 / 2606, the chain-link fence of sector 132, same texture and
    // constants. Read by city.wgsl as `true_sun.w`. MCLA_REARVIEW_CUT=1: the main view's cut.)
    if std::env::var("MCLA_REARVIEW_CUT").is_err() {
        g.true_sun[3] = 1.0;
    }
    let axis = car_rot * Vec3::Z;
    // (diagnostics: MCLA_REARVIEW_LOG=1 prints the mirror's camera each second, the rows as the original's +128)
    static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    if std::env::var("MCLA_REARVIEW_LOG").is_ok() && N.fetch_add(1, std::sync::atomic::Ordering::Relaxed) % 60 == 0 {
        let w = view.inverse();
        eprintln!("rearview eye {:.3} {:.3} {:.3} a {:.4?} b {:.4?} c {:.4?}", eye.x, eye.y, eye.z, w.x_axis.truncate(), w.y_axis.truncate(), w.z_axis.truncate());
    }
    // (the main camera's back axis: minus the clip w row of its view-projection, w = the forward depth)
    let main_cam = Vec3::new(main.cam_pos[0], main.cam_pos[1], main.cam_pos[2]);
    let back = -Vec3::new(main.view_proj[0][3], main.view_proj[1][3], main.view_proj[2][3]).normalize_or_zero();
    let main_eye = if std::env::var_os("MCLA_REARVIEW_DIST_OLD").is_some() { main_cam } else { main_cam + back * 10.0 };
    View { eye, main_eye, axis, frustum: Frustum::from_view_proj(vp), globals: g }
}

/// Draw the mirror into the scene target and depth (after the main view's city pass): the sky (which clears the
/// strip's depth), the city's models of both phases, then the traffic at the LODs `Vehicles::lod_pick` holds
/// for the mirror (the caller swaps them in). Returns the draw count.
pub fn draw(enc: &mut wgpu::CommandEncoder, gpu: &Gpu, world: &crate::world::World, vehicles: &Vehicles, mv: &View) -> usize {
    gpu.queue.write_buffer(&gpu.mirror_globals, 0, bytemuck::bytes_of(&mv.globals));
    // (the scene's size: RENDER SCALE draws it at window x scale)
    let (w, h) = (gpu.scene_w, gpu.scene_h);
    let r = rect(w, h);
    let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("rear-view mirror"),
        // (into the multisampled scene - render/ms.rs; the city's draws write their stencil id as in the main view)
        color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: gpu.scene_color(), resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store } })],
        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment { view: gpu.scene_ds(), depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store }), stencil_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store }) }),
        timestamp_writes: None,
        occlusion_query_set: None,
    });
    pass.set_viewport(r[0], r[1], r[2], r[3], 0.0, 1.0);
    let (x0, y0) = (r[0].round() as u32, r[1].round() as u32);
    let (x1, y1) = (((r[0] + r[2]).round() as u32).min(w), ((r[1] + r[3]).round() as u32).min(h));
    if x1 <= x0 || y1 <= y0 {
        return 0;
    }
    pass.set_scissor_rect(x0, y0, x1 - x0, y1 - y0);
    let mut draws = 0usize;
    // (diagnostics: MCLA_REARVIEW_LIST=1 lists once, at the 200th mirror frame, every far-model batch in the
    // mirror's view - its index count, centre, distance, and whether the port shows it - for matching against
    // the original's recorded VSCityLOD draws)
    static LISTED: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let list_now = std::env::var("MCLA_REARVIEW_LIST").is_ok() && LISTED.fetch_add(1, std::sync::atomic::Ordering::Relaxed) == 200;
    if list_now {
        for (sid, sec) in world.far_models_by_sector() {
            let shown = world.sets.values().any(|s| std::ptr::eq(s, sec));
            if let Some(si) = world.index.sectors.get(sid) {
                let (mn, mx) = (Vec3::from(si.min), Vec3::from(si.max));
                if mv.frustum.intersects_sphere((mn + mx) * 0.5, (mx - mn).length() * 0.5) {
                    eprintln!("rvlist sectorsphere {}", sid);
                }
                eprintln!("rvlist box sector {} min {:.1} {:.1} {:.1} max {:.1} {:.1} {:.1}", sid, si.min[0], si.min[1], si.min[2], si.max[0], si.max[1], si.max[2]);
            }
            for b in sec.batches.iter() {
                eprintln!("rvlist all far sector {} {} n {}", sid, b.shader, b.count);
                let (c, r) = ((b.min + b.max) * 0.5, (b.max - b.min).length() * 0.5);
                if mv.frustum.intersects_sphere(c, r) && !mv.frustum.intersects_aabb(b.min, b.max) {
                    eprintln!("rvlist sphere-only far sector {} {} n {}", sid, b.shader, b.count);
                }
            }
            for b in sec.batches.iter().filter(|b| mv.frustum.intersects_aabb(b.min, b.max)) {
                let c = (b.min + b.max) * 0.5;
                let sd = world.index.sectors.get(sid).map_or(-1.0, |si| sector_distance(si.min, si.max, mv.main_eye));
                eprintln!("rvlist far sector {} {} n {} centre {:.0} {:.0} {:.0} dist {:.0} shown {} loaded {} d268 {:.1}", sid, b.shader, b.count, c.x, c.y, c.z, c.distance(mv.eye), shown, world.loaded.contains_key(&sid), sd);
            }
        }
        for (sid, sec) in world.loaded.iter() {
            for b in sec.batches.iter() {
                let c = (b.min + b.max) * 0.5;
                let seen = mv.frustum.intersects_aabb(b.min, b.max);
                eprintln!("rvlist det{} sector {} {} n {} centre {:.0} {:.0} {:.0} dist {:.0} blend {} interior {}", if seen { "" } else { "out" }, sid, b.shader, b.count, c.x, c.y, c.z, c.distance(mv.eye), matches!(b.mode, MatMode::Blend | MatMode::StoreGlass), b.interior);
            }
        }
    }
    // the sky: the dome by ps_main_cheap, at the far depth, the test off - the strip cleared
    pass.set_bind_group(0, &gpu.mirror_bg, &[]);
    if let Some(tb) = gpu.sky_hat.tex_bg.as_ref() {
        pass.set_pipeline(&gpu.pipeline_mirror_sky);
        pass.set_bind_group(1, tb, &[]);
        pass.set_bind_group(2, &gpu.refl_bg, &[]);
        pass.draw(0..3, 0..1);
        draws += 1;
    }
    // the city: the main view's programs, both phases (opaque, then the blended - shop panes with depth writes)
    let lens = ((FOV_Y_DEG.to_radians() * 0.5).tan() / 0.637).clamp(0.3, 1.2);
    let models = models(world, mv.main_eye);
    pass.set_bind_group(3, &gpu.identity_obj.bg, &[]);
    for phase in 0..2 {
        pass.set_pipeline(if phase == 0 { &gpu.pipeline_opaque } else { &gpu.pipeline_blend });
        let mut pane_pipe = false;
        let mut last_mode = u32::MAX;
        for &(sec, own_obj, sphere) in &models {
            let (Some(vb), Some(ib)) = (sec.vb.as_ref(), sec.ib.as_ref()) else { continue };
            if sphere.is_some_and(|(c, r)| !mv.frustum.intersects_sphere(c, r)) {
                continue;
            }
            let mut bound = false;
            let mut last_bind: *const wgpu::BindGroup = std::ptr::null();
            let mut last_obj = 0u8;
            if !own_obj {
                pass.set_bind_group(3, &gpu.identity_obj.bg, &[]);
            }
            for b in &sec.batches {
                let blend = matches!(b.mode, MatMode::Blend | MatMode::StoreGlass);
                if blend != (phase == 1) {
                    continue;
                }
                // (a far model is drawn whole. A detailed sector's drawables one by one, each when its sphere - the
                // centre and half diagonal of its box - is in the mirror's view: F-76875 keeps flag 0x400 only
                // when F-29884 passes it against the mirror's viewport at ctx+2000; the distance gate after it,
                // ctx+3188 = mirror+40 = 0.0, is off, so nothing is left out for being small or far. Batches without
                // stored drawables (the interior sets) by their box with the main pass's small-object culling.)
                let mut runs: Vec<(u32, u32)> = Vec::new();
                if sphere.is_none() {
                    if b.parts.is_empty() {
                        if !mv.frustum.intersects_aabb(b.min, b.max) {
                            continue;
                        }
                        let ext = (b.max - b.min).length();
                        if ext < ((b.min + b.max) * 0.5).distance(mv.eye) * 0.012 * lens {
                            continue;
                        }
                    } else {
                        for p in &b.parts {
                            // (the drawable's bucket is the low three bits of its shader word, F-21213; for
                            // pass 0x01000000 F-65459 draws buckets 0, 2 and 4 - bucket 1, the blended windows
                            // and signs, only for passes with a bit of 0x1F0)
                            if !mirror_bucket(p.4) {
                                continue;
                            }
                            if mv.frustum.intersects_sphere(p.2, p.3) {
                                if list_now {
                                    eprintln!("rvlist part {} n {} centre {:.0} {:.0} {:.0}", b.shader, p.1, p.2.x, p.2.y, p.2.z);
                                }
                                match runs.last_mut() {
                                    Some(r) if r.0 + r.1 == p.0 => r.1 += p.1,
                                    _ => runs.push((p.0, p.1)),
                                }
                            }
                        }
                        if runs.is_empty() {
                            continue;
                        }
                    }
                }
                if let Some(o) = sec.obj.as_ref().filter(|_| own_obj) {
                    let want = if b.interior { 2 } else { 1 };
                    if want != last_obj {
                        pass.set_bind_group(3, if b.interior { &o.1.bg } else { &o.0.bg }, &[]);
                        last_obj = want;
                    }
                }
                if !bound {
                    pass.set_vertex_buffer(0, vb.slice(..));
                    pass.set_index_buffer(ib.slice(..), wgpu::IndexFormat::Uint32);
                    bound = true;
                }
                let bp = std::sync::Arc::as_ptr(&b.bind);
                if bp != last_bind {
                    pass.set_bind_group(1, b.bind.as_ref(), &[]);
                    last_bind = bp;
                }
                if b.mode as u32 != last_mode {
                    pass.set_bind_group(2, &gpu.mat_bgs[&(b.mode as u32)], &[]);
                    last_mode = b.mode as u32;
                }
                if phase == 1 {
                    let want_pane = matches!(b.mode, MatMode::StoreGlass);
                    if want_pane != pane_pipe {
                        pass.set_pipeline(if want_pane { &gpu.pipeline_pane } else { &gpu.pipeline_blend });
                        pane_pipe = want_pane;
                    }
                }
                if runs.is_empty() {
                    pass.draw_indexed(b.first..b.first + b.count, 0, 0..1);
                    draws += 1;
                    if list_now {
                        eprintln!("rvlist drawn {} n {}", b.shader, b.count);
                    }
                } else {
                    for r in &runs {
                        pass.draw_indexed(r.0..r.0 + r.1, 0, 0..1);
                        draws += 1;
                    }
                }
            }
            pass.set_bind_group(3, &gpu.identity_obj.bg, &[]);
        }
        if phase == 0 {
            // the traffic (the pass 0x01000000 of F-27833): opaque parts here, the glass after the city's blended
            pass.set_pipeline(&gpu.pipeline_opaque);
            vehicles.draw(&mut pass, gpu, false, &mut draws, None, Some(&mv.frustum));
            pass.set_bind_group(3, &gpu.identity_obj.bg, &[]);
        }
    }
    pass.set_pipeline(&gpu.pipeline_blend);
    vehicles.draw(&mut pass, gpu, true, &mut draws, None, Some(&mv.frustum));
    draws
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_at_the_original_size_and_scaled() {
        assert_eq!(rect(1280, 720), [960.0, 57.0, 256.0, 51.0]);
        assert_eq!(rect(1920, 1080), [1440.0, 85.5, 384.0, 76.5]);
    }

    /// The projection recorded in the original's mirror pass (VSCityLOD c8 .. c11 of rv1): P00 1.51322, P11 7.59575.
    #[test]
    fn projection_as_recorded() {
        let p = proj(0.2);
        assert!((p.x_axis.x - 1.51322).abs() < 2e-4, "{}", p.x_axis.x);
        assert!((p.y_axis.y - 7.59575).abs() < 2e-4, "{}", p.y_axis.y);
    }

    /// The camera recorded in rv1 (the viewport object's +128 read from the running original): eye (819.610,
    /// 19.072, -808.409), rows a (0.8800, -0.0429, 0.4730), b (0.0184, 0.9982, 0.0564), c (-0.4746, -0.0410,
    /// 0.8792) - from the car's matrix with a and c turned round.
    #[test]
    fn camera_as_recorded() {
        let (a, b, c) = (Vec3::new(0.8800, -0.0429, 0.4730), Vec3::new(0.0184, 0.9982, 0.0564), Vec3::new(-0.4746, -0.0410, 0.8792));
        // the car's own rows: a and c the other way round
        let car = Quat::from_mat3(&glam::Mat3::from_cols(-a, b, -c).orthonormalize_hack());
        let (eye, view) = camera(Vec3::new(819.610, 17.572, -808.409), car);
        assert!((eye - Vec3::new(819.610, 19.072, -808.409)).length() < 1e-3);
        let world = view.inverse();
        for (got, want) in [(world.x_axis.truncate(), a), (world.y_axis.truncate(), b), (world.z_axis.truncate(), c)] {
            assert!((got - want).length() < 3e-3, "{got:?} {want:?}");
        }
    }

    /// F-75080's mirror rule, with the o14s recording's sectors (record, state, +268): 227 (3, 0.0) and 228
    /// (3, 10.2) in detail; 229 (3, 40.5), 225 (3, 87.0), 218 (3, 180.0) and 219 (2, 321.6) as far models.
    #[test]
    fn buckets_in_the_mirror() {
        assert!(mirror_bucket(0) && mirror_bucket(2) && mirror_bucket(3) && mirror_bucket(4));
        assert!(!mirror_bucket(1));
    }

    #[test]
    fn sectors_in_the_mirror() {
        assert!(mirror_detail(true, 0.0));
        assert!(mirror_detail(true, 10.2));
        assert!(!mirror_detail(true, 20.0));
        assert!(!mirror_detail(true, 40.5));
        assert!(!mirror_detail(false, 5.0));
        assert_eq!(sector_distance([0.0, 0.0, 0.0], [10.0, 5.0, 10.0], Vec3::new(5.0, 100.0, 5.0)), 0.0);
        assert_eq!(sector_distance([0.0, 0.0, 0.0], [10.0, 5.0, 10.0], Vec3::new(13.0, -50.0, 14.0)), 4.0);
    }

    #[test]
    fn traffic_lod_in_the_mirror() {
        // (most types: 40 / 155 in the main view -> 5 / 19.375 here; LOD 0 drawn as 1)
        let s = [40.0, 155.0];
        assert_eq!(mirror_lod(3.0, s), 1);
        assert_eq!(mirror_lod(19.0, s), 1);
        assert_eq!(mirror_lod(19.375, s), 2);
        assert_eq!(mirror_lod(125.0, s), 2);
        assert_eq!(mirror_lod(125.01, s), -1);
    }

    trait Ortho {
        fn orthonormalize_hack(self) -> glam::Mat3;
    }
    impl Ortho for glam::Mat3 {
        /// (the recorded rows carry four digits: make them a rotation)
        fn orthonormalize_hack(self) -> glam::Mat3 {
            let x = self.x_axis.normalize();
            let z = x.cross(self.y_axis).normalize();
            let y = z.cross(x);
            glam::Mat3::from_cols(x, y, z)
        }
    }
}
