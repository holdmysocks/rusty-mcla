//! The race markers: the green chevron the original stands over every computer racer in a race
//! (the `icon_racer` mesh, tinted by the `mcUIClass` tune of `tune/ui`). Decode:
//! `docs/reference/ui/racer_marker.md`; the functions are the icon manager's (vtable 0x82025BDC):
//! `F-54513` (who gets one), `F-50704` (colour, fade), `F-52921` (matrix),
//! `F-34178` / `F-34858` / `F-23750` (queue, far first, drawn through
//! `F-61799` with the scene's depth: the GPS map icons' x-ray technique - `hudmap_gpu.rs`).

use glam::{Mat4, Vec3, Vec4};
use std::collections::HashMap;
use std::path::Path;

/// The `mcUIClass` block of `tune/ui` (`tune/ui/0x75b995ab`; fields registered by
/// `F-61858`, the icon manager holds them 20 bytes further on).
#[derive(Clone, Debug)]
pub struct MarkTune {
    /// HookmanIconOffsetScale: the marker's centre stands this many times the car's bound
    /// height above the car (1.9)
    pub offset_scale: f32,
    /// DepthTestRacerMarker: drawn against the scene's depth (dimmed behind it)
    pub depth_test: bool,
    /// RacerMarkerScale (1, 1.4, 1), applied along the world's axes
    pub scale: Vec3,
    /// RacerMarkerColor, PaybackRacerColor, PaybackBuddyColor
    pub color: [f32; 4],
    pub payback_racer: [f32; 4],
    pub payback_buddy: [f32; 4],
    /// RacerMarkerFadeInBegin / Span, FadeOutBegin / Span (m from the camera)
    pub fade_in: (f32, f32),
    pub fade_out: (f32, f32),
    /// The free-roam 3D icons (hookicons.rs): HookmanIconScale (1.4, the icon frame's scale),
    /// HookmanIconTranslatePivot ((0, 1.07, -0.2), along the frame's axes), IconFadeoutDist
    /// (400 m: no icon beyond)
    pub icon_scale: f32,
    pub icon_pivot: Vec3,
    pub icon_fadeout: f32,
}

impl Default for MarkTune {
    /// The values of the file (what the game ships).
    fn default() -> MarkTune {
        MarkTune {
            offset_scale: 1.9,
            depth_test: true,
            scale: Vec3::new(1.0, 1.4, 1.0),
            color: [0.529412, 0.784314, 0.0, 1.0],
            payback_racer: [0.529412, 0.784314, 0.0, 1.0],
            payback_buddy: [1.0, 0.0, 0.0, 1.0],
            fade_in: (20.0, 45.0),
            fade_out: (175.0, 40.0),
            icon_scale: 1.4,
            icon_pivot: Vec3::new(0.0, 1.07, -0.2),
            icon_fadeout: 400.0,
        }
    }
}

impl MarkTune {
    pub fn load(data_dir: &str) -> MarkTune {
        let mut m: HashMap<String, Vec<f32>> = HashMap::new();
        if let Ok(rd) = std::fs::read_dir(Path::new(data_dir).join("tune/ui")) {
            for e in rd.flatten() {
                let Ok(t) = std::fs::read_to_string(e.path()) else { continue };
                if !t.contains("mcUIClass {") {
                    continue;
                }
                for l in t.lines() {
                    let mut tok = l.split_whitespace();
                    let Some(k) = tok.next() else { continue };
                    let v: Vec<f32> = tok.filter_map(|x| x.parse().ok()).collect();
                    if !v.is_empty() {
                        m.insert(k.to_string(), v);
                    }
                }
            }
        }
        let d = MarkTune::default();
        let f = |k: &str, def: f32| m.get(k).and_then(|v| v.first().copied()).unwrap_or(def);
        let c = |k: &str, def: [f32; 4]| m.get(k).filter(|v| v.len() >= 4).map(|v| [v[0], v[1], v[2], v[3]]).unwrap_or(def);
        MarkTune {
            offset_scale: f("HookmanIconOffsetScale", d.offset_scale),
            depth_test: f("DepthTestRacerMarker", 1.0) != 0.0,
            scale: m.get("RacerMarkerScale").filter(|v| v.len() >= 3).map_or(d.scale, |v| Vec3::new(v[0], v[1], v[2])),
            color: c("RacerMarkerColor", d.color),
            payback_racer: c("PaybackRacerColor", d.payback_racer),
            payback_buddy: c("PaybackBuddyColor", d.payback_buddy),
            fade_in: (f("RacerMarkerFadeInBegin", d.fade_in.0), f("RacerMarkerFadeInSpan", d.fade_in.1)),
            fade_out: (f("RacerMarkerFadeOutBegin", d.fade_out.0), f("RacerMarkerFadeOutSpan", d.fade_out.1)),
            icon_scale: f("HookmanIconScale", d.icon_scale),
            icon_pivot: m.get("HookmanIconTranslatePivot").filter(|v| v.len() >= 3).map_or(d.icon_pivot, |v| Vec3::new(v[0], v[1], v[2])),
            icon_fadeout: f("IconFadeoutDist", d.icon_fadeout),
        }
    }

    /// The marker's colour for a racer of icon type `icon` (`Racer_SetIconType`): 10 and 18 (the
    /// payback client) PaybackRacerColor, 13 (the payback chasers) PaybackBuddyColor, the rest
    /// RacerMarkerColor.
    pub fn colour(&self, icon: u8) -> [f32; 4] {
        match icon {
            10 | 18 => self.payback_racer,
            13 => self.payback_buddy,
            _ => self.color,
        }
    }

    /// The colour's alpha at `d` metres from the camera from the tune colour's alpha `a`, None = not
    /// drawn: nothing nearer than FadeInBegin, in over FadeInSpan, out from FadeOutBegin over
    /// FadeOutSpan, nothing beyond. Icon type 18 (the payback client being chased) is always drawn,
    /// its alpha set to 1 (not the tune's). (F-50704, the order of its tests and products.)
    pub fn fade(&self, icon: u8, d: f32, a: f32) -> Option<f32> {
        if icon == 18 {
            return Some(1.0);
        }
        let (ib, is) = self.fade_in;
        let (ob, os) = self.fade_out;
        if ib <= d && d <= is + ib {
            Some((d - ib) / is * a)
        } else if ob <= d && d <= os + ob {
            Some((1.0 - (d - ob) / os) * a)
        } else if is + ib < d && d < ob {
            Some(a)
        } else {
            None
        }
    }

    /// The marker's world matrix (F-52921) for a car standing at `car` (its matrix's translation)
    /// whose composite bound is `height` tall, seen by a camera whose matrix has the z row `cam_z`
    /// (pointing back, the camera looks along -z): f = -cam_z, right = |f x Y|, up = |right x f|
    /// (the vector unit's refined reciprocal root, 0 for a zero length), the flat `icon_racer` quad
    /// turned upright by the quarter turn about x (F-67091: rows (1,0,0), (0,c,s), (0,-s,c) of
    /// pi/2), then the tune's matrix (unit rows times RacerMarkerScale), centred
    /// `offset_scale x height` above the car (fused). RacerMarkerTranslate only reaches the
    /// translation row, which the position then overwrites: it has no effect. Every lane, product
    /// and sum in the original's order; the translation's w is 1 here (the original leaves the
    /// tune's w there, the draw reads 4x3).
    pub fn matrix(&self, cam_z: Vec3, car: Vec3, height: f32) -> Mat4 {
        type V = [f32; 4];
        let mul = |a: V, b: V| -> V { [a[0] * b[0], a[1] * b[1], a[2] * b[2], a[3] * b[3]] };
        let add = |a: V, b: V| -> V { [a[0] + b[0], a[1] + b[1], a[2] + b[2], a[3] + b[3]] };
        // vnmsubfp as the recomp: -(a b - c)
        let nmsub = |a: V, b: V, c: V| -> V {
            let p = mul(a, b);
            [-(p[0] - c[0]), -(p[1] - c[1]), -(p[2] - c[2]), -(p[3] - c[3])]
        };
        let yzx = |a: V| -> V { [a[1], a[2], a[0], a[3]] };
        let zxy = |a: V| -> V { [a[2], a[0], a[1], a[3]] };
        let splat = |x: f32| -> V { [x; 4] };
        // vmsum3fp (the x86 dot product's order: (0 + z z) + (y y + x x)), the reciprocal root
        // (+inf -> 0), one refinement step y + (1 - s y y) (y / 2)
        let unit = |v: V| -> V {
            let s = (0.0 + v[2] * v[2]) + (v[1] * v[1] + v[0] * v[0]);
            let e = 1.0 / s.sqrt();
            let e = if e == f32::INFINITY { 0.0 } else { e };
            let k = -(s * (e * e) - 1.0);
            let y = k * (0.5 * e) + e;
            mul(v, splat(y))
        };
        let up: V = [0.0, 1.0, 0.0, 0.0];
        let f: V = [0.0 - cam_z.x, 0.0 - cam_z.y, 0.0 - cam_z.z, 0.0 - 0.0];
        let r = unit(nmsub(yzx(up), zxy(f), mul(zxy(up), yzx(f))));
        let u = unit(nmsub(yzx(f), zxy(r), mul(zxy(f), yzx(r))));
        // the quarter turn: cos / sin of pi/2 (f32) by the guest's libm, rounded to f32
        let half_pi = std::f32::consts::FRAC_PI_2 as f64;
        let (c, sn) = (crate::physics::exact::guest_cos(half_pi) as f32, crate::physics::exact::guest_sin(half_pi) as f32);
        let q: [[f32; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, c, sn], [0.0, -sn, c]];
        let s: V = [self.scale.x, self.scale.y, self.scale.z, 0.0];
        let m = [mul([1.0, 0.0, 0.0, 0.0], s), mul([0.0, 1.0, 0.0, 0.0], s), mul([0.0, 0.0, 1.0, 0.0], s)];
        let row = |i: usize| -> Vec4 {
            let t = add(add(mul(r, splat(q[i][0])), mul(u, splat(q[i][1]))), mul(f, splat(q[i][2])));
            let o = add(add(mul(m[0], splat(t[0])), mul(m[1], splat(t[1]))), mul(m[2], splat(t[2])));
            Vec4::from_array(o)
        };
        let y = self.offset_scale.mul_add(height, car.y);
        Mat4::from_cols(row(0), row(1), row(2), Vec4::new(car.x, y, car.z, 1.0))
    }
}

/// The distance from the camera at `cam` to `p` as the original takes it (F-50704 and the queue
/// F-34178): dy dy, then dz dz and dx dx fused on, the single root.
pub fn distance(cam: Vec3, p: Vec3) -> f32 {
    let (dx, dy, dz) = (p.x - cam.x, p.y - cam.y, p.z - cam.z);
    dx.mul_add(dx, dz.mul_add(dz, dy * dy)).sqrt()
}

/// One marker to draw this frame: world matrix, colour (alpha faded), distance from the camera
/// to its centre.
#[derive(Clone, Copy, Debug)]
pub struct Mark {
    pub m: Mat4,
    pub colour: [f32; 4],
    pub dist: f32,
}

/// A racer the markers consider: where its car stands, its bound's height, its icon type.
pub struct Racer {
    pub pos: Vec3,
    pub height: f32,
    pub icon: u8,
}

/// How far below its tyre's crown the lowest wheel reaches in the car's composite bound as the
/// running game holds it. Read live from four cars: the wheel parts themselves are boxes of
/// +-crown radius about the hub, yet the composite's min y = hub - crown - 0.500 on the S281 and
/// the R8 (to 0.1 mm), 0.524 on the Murcielago and 0.508 on the Ducati 999R - as if the wheels
/// hung about half a metre down when the box was made (it never changes afterwards).
/// *(approx.: where this comes from was not found; 0.5 leaves those two 2.4 / 1.1 cm short,
/// 4.6 / 2.1 cm in the marker's height)*.
pub const WHEEL_BOX_DROP: f32 = 0.5;

/// The height of the car's composite bound as the running game has it (`F-98453` +16 y
/// minus +32 y), a constant of the car from its set-up on (100 live samples, at rest and racing):
/// the top is the body hull's own box (+0x10 y, margin in - S281 1.4117 = its hull part's 1.412),
/// the bottom the wheel parts' - each at the model's hub (the file's part matrix), its tyre's
/// crown radius (`vehsim::crown` of the fitted TireRadius / TireWidth, front = -z) and
/// `WHEEL_BOX_DROP` below.
pub fn bound_height(c: &crate::vehicles::CarInstance) -> f32 {
    let Some(h) = c.model.params.hull.as_ref() else { return c.model.max.y - c.model.min.y };
    let top = h.bbox.map_or(c.model.max.y, |(hi, _)| hi.y);
    let tyres = match (c.setup.as_ref(), c.model.simtunes.as_ref()) {
        (Some(f), _) => Some((f.radius, f.width)),
        (None, Some(s)) => Some(([s.base.wheel_front.radius, s.base.wheel_back.radius], [s.base.wheel_front.width, s.base.wheel_back.width])),
        _ => None,
    };
    let Some((r, w)) = tyres.filter(|_| !h.wheel_parts.is_empty()) else { return top - c.model.min.y };
    let bottom = box_bottom(&h.wheel_parts, r, w);
    if std::env::var("MCLA_MARK_LOG").is_ok() {
        log::info!("race marker bound {}: top {:.4} bottom {:.4} h {:.4} tyres {:?} {:?}", c.model.name, top, bottom, top - bottom, r, w);
    }
    top - bottom
}

/// The lowest wheel part's box bottom: hub - crown radius (tyre radius `r`, width `w`; front
/// = index 0, the parts at -z) - `WHEEL_BOX_DROP`.
pub fn box_bottom(wheel_parts: &[Vec3], r: [f32; 2], w: [f32; 2]) -> f32 {
    wheel_parts.iter().map(|p| {
        let a = if p.z < 0.0 { 0 } else { 1 };
        p.y - crate::physics::vehsim::crown(r[a], w[a], 0.0)
    }).fold(f32::MAX, f32::min) - WHEEL_BOX_DROP
}

/// This frame's markers, far first (`F-13438` sorts the queue by distance, descending).
/// `cam_z` is the camera matrix's z row (pointing back).
pub fn marks(t: &MarkTune, cam: Vec3, cam_z: Vec3, racers: &[Racer]) -> Vec<Mark> {
    let mut out: Vec<Mark> = racers
        .iter()
        .filter_map(|r| {
            let d = distance(cam, r.pos);
            let mut colour = t.colour(r.icon);
            colour[3] = t.fade(r.icon, d, colour[3])?;
            // (the queue F-34178 keeps the distance to the marker's centre, which it sorts by)
            let m = t.matrix(cam_z, r.pos, r.height);
            Some(Mark { m, colour, dist: distance(cam, m.w_axis.truncate()) })
        })
        .collect();
    out.sort_by(|a, b| b.dist.partial_cmp(&a.dist).unwrap_or(std::cmp::Ordering::Equal));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fade_rule() {
        let t = MarkTune::default();
        assert_eq!(t.fade(1, 10.0, 1.0), None);
        assert_eq!(t.fade(1, 20.0, 1.0), Some(0.0));
        assert!((t.fade(1, 42.5, 1.0).unwrap() - 0.5).abs() < 1e-6);
        assert_eq!(t.fade(1, 100.0, 1.0), Some(1.0));
        assert!((t.fade(1, 195.0, 1.0).unwrap() - 0.5).abs() < 1e-6);
        assert_eq!(t.fade(1, 216.0, 1.0), None);
        assert_eq!(t.fade(18, 500.0, 0.5), Some(1.0));
        assert_eq!(t.colour(13), [1.0, 0.0, 0.0, 1.0]);
    }

    /// The composite box heights read out of the running original (`work/career2/mk/rec`,
    /// marks.py): S281, R8, Murcielago, Ducati 999R.
    #[test]
    fn live_box_heights() {
        let root = std::env::var("MCLA_DATA_DIR").unwrap_or_else(|_| "D:/MCLA/extracted/cache".into());
        let dir = std::path::Path::new(&root).join("resources/vehicle");
        if !dir.exists() {
            return;
        }
        let assets = crate::assets::Assets::new(&root);
        for (name, top_live, bottom_live) in [("vp_sln_s281_06", 1.4117f32, -0.5112f32), ("vp_aud_r8_08", 1.2772, -0.4759), ("vp_lam_murcielago_06", 1.0885, -0.5459), ("vp_duc_999r_06", 1.1482, -0.5071)] {
            let car = crate::vehicle_model::load_car(&assets, &dir.join(name)).unwrap();
            let h = car.hull.as_ref().unwrap();
            let s = &car.simtune.as_ref().unwrap().base;
            let wheels: Vec<Vec3> = h.wheel_parts.iter().map(|p| Vec3::from(*p)).collect();
            let bottom = box_bottom(&wheels, [s.wheel_front.radius, s.wheel_back.radius], [s.wheel_front.width, s.wheel_back.width]);
            println!("{name}: top {:.4} ({top_live}) bottom {:.4} ({bottom_live}) tyres {} {} / {} {}", h.bbox.0[1], bottom, s.wheel_front.radius, s.wheel_front.width, s.wheel_back.radius, s.wheel_back.width);
            assert!((h.bbox.0[1] - top_live).abs() < 5e-4, "{name} top");
            // (S281 and R8 to 0.1 mm; the Murcielago's box reaches 2.4 cm and the Ducati's 1.1 cm
            // lower than the 0.5 rule - WHEEL_BOX_DROP)
            assert!((bottom - bottom_live).abs() < 0.03, "{name} bottom {bottom}");
        }
    }

    /// F-50704 / F-52921 / F-34178 run offline (tools/race_marker_reference.py, 4,000 cases):
    /// queued or not, the matrix (row 3's w aside), the colour, the depth flag and the distance,
    /// every word equal.
    #[test]
    fn markers_match_the_original() {
        let text = include_str!("testdata/race_marker_ref.txt");
        let mut n = 0;
        for line in text.lines() {
            let (a, b) = line.split_once(" -> ").unwrap();
            let w: Vec<u32> = a.split(' ').map(|x| x.parse().unwrap()).collect();
            let f = |i: usize| f32::from_bits(w[i]);
            let v3 = |i: usize| Vec3::new(f(i), f(i + 1), f(i + 2));
            let (cam_z, cam, pos) = (v3(0), v3(3), v3(6));
            let height = f(9) - f(10);
            let c4 = |i: usize| [f(i), f(i + 1), f(i + 2), f(i + 3)];
            let t = MarkTune {
                offset_scale: f(11),
                depth_test: w[35] != 0,
                scale: v3(12),
                fade_in: (f(18), f(19)),
                fade_out: (f(20), f(21)),
                color: c4(22),
                payback_racer: c4(26),
                payback_buddy: c4(30),
                ..MarkTune::default()
            };
            let icon = w[34] as u8;
            let o: Vec<u32> = b.split(' ').map(|x| x.parse().unwrap()).collect();
            let got = marks(&t, cam, cam_z, &[Racer { pos, height, icon }]);
            assert_eq!(got.len() as u32, o[0], "{line}");
            if let Some(m) = got.first() {
                let mut words: Vec<u32> = m.m.to_cols_array().iter().map(|x| x.to_bits()).collect();
                words.truncate(15);
                assert_eq!(words, o[1..16].to_vec(), "matrix {line}");
                assert_eq!(m.colour.map(f32::to_bits).to_vec(), o[17..21].to_vec(), "colour {line}");
                assert_eq!(t.depth_test as u32, o[21], "depth {line}");
                assert_eq!(m.dist.to_bits(), o[22], "distance {line}");
            }
            n += 1;
        }
        assert_eq!(n, 4000);
    }

    #[test]
    fn matrix_faces_the_camera() {
        let t = MarkTune::default();
        // camera looking down -z: the quad spans x and y, 1.4 times taller than wide
        let m = t.matrix(Vec3::new(0.0, 0.0, 1.0), Vec3::new(5.0, 1.0, -20.0), 2.0);
        let a = m.transform_point3(Vec3::new(1.0, 0.0, -1.0));
        let b = m.transform_point3(Vec3::new(-1.0, 0.0, 1.0));
        assert!((a - Vec3::new(6.0, 1.0 + 3.8 + 1.4, -20.0)).length() < 1e-5, "{a:?}");
        assert!((b - Vec3::new(4.0, 1.0 + 3.8 - 1.4, -20.0)).length() < 1e-5, "{b:?}");
    }
}
