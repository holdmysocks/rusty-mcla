//! Cameras: free-fly (debug) and chase (gameplay).

use glam::{Mat4, Quat, Vec3};

/// Orthographic sunlight camera whose XY grid is anchored in world space.
/// Quantizing a center in a view that already looks at that center quantizes zero
/// and cannot stabilize the shadow texels.
pub fn shadow_view_proj(center: Vec3, sun: Vec3, extent: f32, resolution: u32) -> Mat4 {
    let sun = sun.normalize_or(Vec3::Y);
    let up = if sun.y.abs() > 0.95 { Vec3::Z } else { Vec3::Y };
    let right = up.cross(sun).normalize();
    let light_up = sun.cross(right);
    let texel = 2.0 * extent / resolution.max(1) as f32;
    let (x, y) = (right.dot(center), light_up.dot(center));
    let center = center + right * ((x / texel).round() * texel - x)
        + light_up * ((y / texel).round() * texel - y);
    let view = Mat4::look_at_rh(center + sun * 400.0, center, up);
    Mat4::orthographic_rh(-extent, extent, -extent, extent, 1.0, 900.0) * view
}

#[cfg(test)]
mod shadow_tests {
    use super::*;

    #[test]
    fn shadow_texels_stay_anchored_during_sub_texel_camera_motion() {
        let sun = Vec3::new(0.4, 0.8, 0.3).normalize();
        let right = Vec3::Y.cross(sun).normalize();
        let texel = 190.0 / 4096.0;
        let point = Vec3::new(1.0, 2.0, 3.0).extend(1.0);
        let a = shadow_view_proj(Vec3::ZERO, sun, 95.0, 4096) * point;
        let b = shadow_view_proj(right * (texel * 0.25), sun, 95.0, 4096) * point;
        assert!((a.x - b.x).abs() < 1e-6);
        assert!((a.y - b.y).abs() < 1e-6);
        let c = shadow_view_proj(right * (texel * 0.75), sun, 95.0, 4096) * point;
        assert!(((c.x - a.x).abs() - 2.0 / 4096.0).abs() < 1e-6,
            "crossing a grid boundary must advance exactly one shadow texel");
    }

    #[test]
    fn sunlight_projection_remains_finite_at_zenith_and_covers_center() {
        for sun in [Vec3::Y, -Vec3::Y, Vec3::new(0.3, 0.4, 0.5)] {
            let center = Vec3::new(819.0, 17.0, -808.0);
            let vp = shadow_view_proj(center, sun, 95.0, 4096);
            assert!(vp.is_finite());
            let projected = vp * center.extend(1.0);
            assert!(projected.x.abs() < 2.0 / 4096.0);
            assert!(projected.y.abs() < 2.0 / 4096.0);
            assert!((0.0..1.0).contains(&projected.z));
        }
    }
}

#[derive(Debug, Clone)]
pub struct Camera {
    pub pos: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub fov_y: f32,
    pub near: f32,
    /// a finite (near, far) instead of `near` and no far plane: the open GPS map's (`hudmap_gpu::orig_planes`)
    pub planes: Option<(f32, f32)>,
}

impl Default for Camera {
    fn default() -> Self {
        Camera { pos: Vec3::new(1600.0, 30.0, 0.0), yaw: 0.0, pitch: -0.2, fov_y: 65f32.to_radians(), near: 0.3, planes: None }
    }
}

impl Camera {
    pub fn forward(&self) -> Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        Vec3::new(sy * cp, sp, -cy * cp).normalize()
    }
    pub fn right(&self) -> Vec3 {
        self.forward().cross(Vec3::Y).normalize()
    }
    pub fn view(&self) -> Mat4 {
        Mat4::look_to_rh(self.pos, self.forward(), Vec3::Y)
    }
    /// Reverse-Z infinite projection (depth 1 near → 0 far) for precision over km distances.
    pub fn proj(&self, aspect: f32) -> Mat4 {
        if let Some((n, far)) = self.planes {
            // (reverse-Z with both planes: depth 1 at n, 0 at far; beyond far is clipped and culled)
            return Mat4::perspective_rh(self.fov_y, aspect, far, n);
        }
        let f = 1.0 / (self.fov_y * 0.5).tan();
        Mat4::from_cols_array(&[
            f / aspect, 0.0, 0.0, 0.0,
            0.0, f, 0.0, 0.0,
            0.0, 0.0, 0.0, -1.0,
            0.0, 0.0, self.near, 0.0,
        ])
    }
    pub fn view_proj(&self, aspect: f32) -> Mat4 {
        self.proj(aspect) * self.view()
    }
    pub fn rotation(&self) -> Quat {
        Quat::from_rotation_y(self.yaw) * Quat::from_rotation_x(self.pitch)
    }
}

/// Smooth third-person chase camera.
pub struct ChaseCam {
    pub pos: Vec3,
    pub look: Vec3,
    pub distance: f32,
    pub height: f32,
    /// smoothed viewing direction (horizontal)
    pub dir: Vec3,
    /// speed zoom 0..1 (long lens .. wide)
    pub zoom: f32,
    /// up vector (cutscene cameras roll)
    pub up: Vec3,
}

impl Default for ChaseCam {
    fn default() -> Self {
        ChaseCam { pos: Vec3::ZERO, look: Vec3::ZERO, distance: 6.8, height: 2.3, dir: Vec3::ZERO, zoom: 0.0, up: Vec3::Y }
    }
}

impl ChaseCam {
    /// The game's boom camera (`camBoomCS` in tune/camera): a long lens far behind the
    /// car at rest (FOV 25 deg, 9.8 m back) that widens to 65 deg and closes to 63 % of
    /// the distance with speed. Returns the vertical field of view in radians.
    pub fn update(&mut self, target: Vec3, forward: Vec3, velocity: Vec3, boost: f32, dt: f32) -> f32 {
        const OFFSET: (f32, f32) = (1.75, 9.8); // height, distance (Offset 0 1.75 9.8)
        // Focus 0 1.5 -1.2 in the file; the height is taken from the car floor here, which
        // keeps the whole car in frame through the long lens
        const FOCUS: (f32, f32) = (0.95, 1.2);
        const FOV: (f32, f32) = (25.0, 65.0); // SSFovMin / SSFovMax
        const DIST_SCALE: f32 = 0.63; // SSDistScale
        let speed = velocity.length();
        let want_dir = if speed > 3.0 { velocity / speed } else { forward };
        let want_dir = Vec3::new(want_dir.x, 0.0, want_dir.z).normalize_or(forward);
        if self.dir.length_squared() < 0.5 || self.pos.distance(target) > 60.0 {
            self.dir = want_dir;
            self.zoom = 0.0;
        }
        let k = 1.0 - (-dt * 5.0).exp();
        self.dir = self.dir.lerp(want_dir, k).normalize_or(want_dir);
        // speed-sensitive zoom, eased so that gear changes do not pump the lens
        let t = ((speed - 4.0) / 50.0).clamp(0.0, 1.0);
        let want = (t * t * (3.0 - 2.0 * t) + boost * 0.25).min(1.15);
        self.zoom += (want - self.zoom) * (1.0 - (-dt * 2.5).exp());
        let z = self.zoom;
        let dist = OFFSET.1 * (1.0 + (DIST_SCALE - 1.0) * z.min(1.0));
        let height = OFFSET.0 * (1.0 - 0.22 * z.min(1.0));
        self.pos = target - self.dir * dist + Vec3::Y * height;
        self.look = target + Vec3::Y * FOCUS.0 * (1.0 - 0.3 * z.min(1.0)) + self.dir * FOCUS.1;
        (FOV.0 + (FOV.1 - FOV.0) * z).to_radians()
    }
    pub fn view(&self) -> Mat4 {
        Mat4::look_at_rh(self.pos, self.look, self.up)
    }
}
