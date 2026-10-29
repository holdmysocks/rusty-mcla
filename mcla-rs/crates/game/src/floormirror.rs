//! The garage floor's mirrored pass (docs/reference/lighting/gaps_evidence.md "The garage floor's mirror").
//!
//! In the garage the original's planar reflection pass - the 256 x 256 picture the wet road reads (refl.wgsl,
//! `Gpu::reflection_pass`) - draws more than the far city and the sky: after the far models (VSCityLODReflect) come
//! the room's batches with their own programs (PSMultiLightInterior, the floor's VSCityGarageFloor), the sky dome,
//! then the whole car with every car program (work/lighting/cap/gar_top.gpu.bin draws 28 .. 502). The room and the
//! car are drawn with the main view's matrices mirrored about the plane y = gReflectionParams.z: c0 .. c11 of their
//! vertex programs are the main view's times diag(1, -1, 1) with 2 h on y (h = 17.0053 recorded, c3.y = 34.0107, the
//! view-projection's fourth row moved by 2 h x its second), culling turned round (0x18006); every pixel constant is
//! the main view's - the eye where it is - except the shadow collector, an 8 x 8 grey stand-in (no cascade shadows),
//! and the room programs' four dynamic lights, zeroed (their colours are zero in the garage anyway). No fog: the
//! original's fog is a pass over the finished frame. The floor's own draws there face away under the turned culling
//! and leave nothing; the port's city pipelines do not cull, so the floor is left out here.
//! Not drawn here: the pass's VS_TransformLit / PS_Textured draw (24 indices) and its two PSSeedRTNoZ glow
//! triangles - what they are is not read.

use crate::render::{Frustum, Globals, Gpu, MatMode};
use crate::vehicles::Vehicles;
use glam::{Mat4, Vec3};

/// The mirror about the plane y = `h` (y -> 2 h - y).
pub fn mirror(h: f32) -> Mat4 {
    Mat4::from_cols_array(&[1.0, 0.0, 0.0, 0.0, 0.0, -1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 2.0 * h, 0.0, 1.0])
}

/// The pass's globals: the main view's with the view mirrored, no fog, the collector's stand-in (no sun shadows) and
/// the picture marked as not there (it is being drawn).
pub fn globals(main: &Globals, h: f32) -> (Globals, Frustum) {
    let vp = Mat4::from_cols_array_2d(&main.view_proj) * mirror(h);
    let mut g = *main;
    g.view_proj = vp.to_cols_array_2d();
    g.inv_view_proj = vp.inverse().to_cols_array_2d();
    g.fog_color[3] = 0.0;
    g.shadow_params[0] = 0.0;
    g.collector[3] = 0.0;
    (g, Frustum::from_view_proj(vp))
}

/// Draw the room (the interior set the camera stands in, without its floor) and the cars into the reflection
/// picture, over what `Gpu::reflection_pass` left there (its colour and depth). Runs with the plain pipelines
/// (`render::ms::set_plain` in force). Returns the draw count.
pub fn draw(enc: &mut wgpu::CommandEncoder, gpu: &Gpu, world: &crate::world::World, vehicles: &Vehicles, main: &Globals, h: f32) -> usize {
    let (mut g, frustum) = globals(main, h);
    // (into the 256 x 256 reflection picture with pix_center 0: half ITS pixel, the main matrix carrying half a scene
    // pixel already - render::half_pixel)
    if crate::render::half_pixel_on() {
        let r = crate::render::REFL_SIZE as f32;
        let t = glam::Mat4::from_translation(Vec3::new(1.0 / r - 1.0 / gpu.scene_w as f32, -(1.0 / r - 1.0 / gpu.scene_h as f32), 0.0));
        let vp = t * glam::Mat4::from_cols_array_2d(&g.view_proj);
        g.view_proj = vp.to_cols_array_2d();
        g.inv_view_proj = vp.inverse().to_cols_array_2d();
    }
    gpu.queue.write_buffer(&gpu.floor_globals, 0, bytemuck::bytes_of(&g));
    let eye = Vec3::new(main.cam_pos[0], main.cam_pos[1], main.cam_pos[2]);
    let plain = crate::render::ms::PLAIN;
    let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("garage floor mirror"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &gpu.refl_view, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store } })],
        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment { view: &gpu.refl_depth, depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Discard }), stencil_ops: None }),
        timestamp_writes: None,
        occlusion_query_set: None,
    });
    pass.set_bind_group(0, &gpu.floor_bg, &[]);
    let mut draws = 0usize;
    let sets: Vec<&crate::world::GpuSector> = world
        .sets
        .iter()
        .filter(|e| *e.0 >= crate::world::SET_ID && *e.0 < crate::world::FAR_ID)
        .map(|e| e.1)
        .filter(|gs| {
            let (mn, mx) = gs.batches.iter().fold((Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)), |a, b| (a.0.min(b.min), a.1.max(b.max)));
            eye.x > mn.x && eye.x < mx.x && eye.z > mn.z && eye.z < mx.z
        })
        .collect();
    for phase in 0..2 {
        pass.set_pipeline(if phase == 0 { gpu.pipeline_opaque.at(plain) } else { gpu.pipeline_blend.at(plain) });
        let mut pane_pipe = false;
        for sec in &sets {
            let (Some(vb), Some(ib)) = (sec.vb.as_ref(), sec.ib.as_ref()) else { continue };
            let mut bound = false;
            let mut last_obj = 0u8;
            for b in &sec.batches {
                let blend = matches!(b.mode, MatMode::Blend | MatMode::StoreGlass);
                if blend != (phase == 1) || b.shader.to_ascii_lowercase().contains("citygaragefloor") || !frustum.intersects_aabb(b.min, b.max) {
                    continue;
                }
                match sec.obj.as_ref() {
                    Some(o) => {
                        let want = if b.interior { 2 } else { 1 };
                        if want != last_obj {
                            pass.set_bind_group(3, if b.interior { &o.1.bg } else { &o.0.bg }, &[]);
                            last_obj = want;
                        }
                    }
                    None if last_obj != 3 => {
                        pass.set_bind_group(3, &gpu.identity_obj.bg, &[]);
                        last_obj = 3;
                    }
                    None => {}
                }
                if !bound {
                    pass.set_vertex_buffer(0, vb.slice(..));
                    pass.set_index_buffer(ib.slice(..), wgpu::IndexFormat::Uint32);
                    bound = true;
                }
                pass.set_bind_group(1, b.bind.as_ref(), &[]);
                pass.set_bind_group(2, &gpu.mat_bgs[&(b.mode as u32)], &[]);
                if phase == 1 {
                    let want_pane = matches!(b.mode, MatMode::StoreGlass);
                    if want_pane != pane_pipe {
                        pass.set_pipeline(if want_pane { gpu.pipeline_pane.at(plain) } else { gpu.pipeline_blend.at(plain) });
                        pane_pipe = want_pane;
                    }
                }
                pass.draw_indexed(b.first..b.first + b.count, 0, 0..1);
                draws += 1;
            }
        }
        if phase == 0 {
            // the cars, opaque parts (their own pipelines follow `ms::current`, the plain one here)
            pass.set_pipeline(gpu.pipeline_opaque.at(plain));
            vehicles.draw(&mut pass, gpu, false, &mut draws, None, Some(&frustum));
        }
    }
    pass.set_pipeline(gpu.pipeline_blend.at(plain));
    vehicles.draw(&mut pass, gpu, true, &mut draws, None, Some(&frustum));
    draws
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The recorded room draw of cap/gar_top (mirror pass 109 against main pass 2185, VSCityBumpSpec n 36792):
    /// c8 .. c11 of the mirrored draw = the main view's times the mirror about h = 17.005342 (gReflectionParams.z).
    #[test]
    fn mirrored_view_projection_as_recorded() {
        let main = Mat4::from_cols_array(&[-1.542848, -0.002393, -0.058065, -0.058064, 0.0, 2.747169, -0.015, -0.014999, -0.089746, 0.041141, 0.998217, 0.9982, 1255.446045, -13.512441, 863.742188, 863.827759]);
        let want = Mat4::from_cols_array(&[-1.542848, -0.002393, -0.058065, -0.058064, 0.0, -2.747169, 0.015, 0.014999, -0.089746, 0.041141, 0.998217, 0.9982, 1255.446045, 79.920639, 863.232056, 863.317627]);
        let got = main * mirror(17.005342);
        for (a, b) in got.to_cols_array().iter().zip(want.to_cols_array().iter()) {
            assert!((a - b).abs() < 2e-3 * b.abs().max(1.0), "{got:?}");
        }
    }
}
