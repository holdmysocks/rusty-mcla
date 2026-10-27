//! Step 19g: the lightning on a car hit by Reverse Steering, Stop or the Disrupter (and their
//! megas), as the original makes it (SAVE_WORK.md "Step 19g"):
//! - the start (the types' +12: sub_8237D110 Reverse, sub_8237E978 Stop, sub_8237D920 Disrupter -
//!   not on the local player, who gets the screen tint) -> sub_82375E58: ten bolts (the car's
//!   render object +2096, 336 bytes each, set up by sub_827496B0 from the manager's
//!   m_XSize .. m_SampleRangeMinMax) and per bolt two arc styles, major and minor (+5456 / +6096,
//!   64 bytes, sub_822ED948 from the type's ma_ / mi_ values);
//! - every frame (the types' +32 -> sub_82375EB8): the bolt's frame = the car's matrix with its
//!   rows times the bound's box maximum z (the third negated) and m_Offset added in world axes;
//!   the bolt turns (sub_82749B48) and casts five rays at the car's body - the arc's five control
//!   points; the styles' clocks run (sub_822EDAF0);
//! - the draw (the types' +40 -> sub_82376070 -> sub_82749590 -> sub_822EE490 / sub_822EDD80):
//!   per bolt and style a camera-facing strip of Iterations + 1 pairs along a Bezier curve
//!   through jittered points, lightning.fxc / lightning.dds, blend source alpha, one, depth
//!   tested, not written, the alpha x time left / ActiveTime.

use glam::{Quat, Vec3};

/// The manager's bolt settings (mcPowerUpMgr +12 .. +68, sub_82740220) and m_Offset (+304).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoltTune {
    pub size: Vec3,
    pub life: [f32; 2],
    pub range: [f32; 2],
    pub arc_speed: [f32; 2],
    pub start_theta: [f32; 2],
    pub end_theta: [f32; 2],
    pub sample_range: [f32; 2],
    pub offset: Vec3,
}

/// An arc style (the type's ma_ / mi_ block, 112 bytes: +0 ColorMin, +16 ColorMax, +32
/// EndColor, +48 ArcGrowth, +56 GrowthSpeed, +64 FluxTangentRange, +72 FluxNormalRange, +80
/// FlickerSpeed, +88 Thickness, +92 TexAnimRate, +96 Iterations).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ArcTune {
    pub color_min: [f32; 4],
    pub color_max: [f32; 4],
    pub end_color: [f32; 4],
    pub arc_growth: [f32; 2],
    pub growth_speed: [f32; 2],
    pub flux_tangent: [f32; 2],
    pub flux_normal: [f32; 2],
    pub flicker_speed: [f32; 2],
    pub thickness: f32,
    pub tex_anim_rate: f32,
    pub iterations: usize,
}

/// The game's random numbers (0x82860880): multiply with carry by 0x5CDCFAA7; a number is the
/// low word's 23 bits x 2^-23.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mwc {
    pub lo: u32,
    pub carry: u32,
}

impl Mwc {
    pub fn next(&mut self) -> u32 {
        let x = self.lo as u64 * 0x5CDC_FAA7 + self.carry as u64;
        self.lo = x as u32;
        self.carry = (x >> 32) as u32;
        x as u32
    }
    pub fn unit(&mut self) -> f32 {
        (self.next() & 0x7F_FFFF) as f32 * 1.192_092_9e-7
    }
    /// min + (max - min) x a random number (the "MinMax" pairs)
    pub fn range(&mut self, r: [f32; 2]) -> f32 {
        let u = self.unit();
        (r[1] - r[0]) * u + r[0]
    }
}

/// One bolt (sub_827496B0's record): +0 SampleRange, +4 theta, +8 ArcSpeed, +12 Range, +16
/// EndTheta, +20 life, the five control points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bolt {
    pub sample: f32,
    pub theta: f32,
    pub speed: f32,
    pub range: f32,
    pub end: f32,
    pub life: f32,
    pub points: [Vec3; 5],
    /// the frame's rows the draw jitters along: +64 (row 0 of the turned frame) and +48 (row 2)
    pub dir_a: Vec3,
    pub dir_b: Vec3,
}

/// An arc style's state (sub_822ED948's record): +0 the colour, +32 the texture's offset,
/// +40 the shape's seed, +48 the growth clock, +20 its rate, +52 the flicker clock, +28 its rate.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Arc {
    pub color: [f32; 4],
    pub growth_rate: f32,
    pub flicker_rate: f32,
    pub tex: f32,
    pub seed: u32,
    pub growth: f32,
    pub flicker: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Lightning {
    pub bolts: [Bolt; 10],
    /// per bolt: major, minor
    pub arcs: [[Arc; 2]; 10],
    pub tune: BoltTune,
    pub styles: [ArcTune; 2],
    pub rng: Mwc,
}

fn bolt_init(t: &BoltTune, rng: &mut Mwc) -> Bolt {
    // (sub_827496B0: the order of the draws)
    let speed = rng.range(t.arc_speed);
    let theta = rng.range(t.start_theta);
    let end = rng.range(t.end_theta);
    let sample = rng.range(t.sample_range);
    let range = rng.range(t.range);
    let life = rng.range(t.life);
    Bolt { sample, theta, speed, range, end, life, points: [Vec3::ZERO; 5], dir_a: Vec3::X, dir_b: Vec3::Z }
}

fn arc_init(s: &ArcTune, rng: &mut Mwc) -> Arc {
    // (sub_822ED948: +16 ArcGrowth (unread by the draw), +20 1 / GrowthSpeed, +28 1 /
    // FlickerSpeed, +24 FluxTangent (unread), +40 a 31-bit seed, the clocks 0)
    let _growth = rng.range(s.arc_growth);
    let growth_rate = 1.0 / rng.range(s.growth_speed);
    let flicker_rate = 1.0 / rng.range(s.flicker_speed);
    let _flux = rng.range(s.flux_tangent);
    let seed = rng.next() & 0x7FFF_FFFF;
    Arc { color: s.color_min, growth_rate, flicker_rate, tex: 0.0, seed, growth: 0.0, flicker: 0.0 }
}

impl Lightning {
    /// sub_82375E58: ten bolts, each with its two styles.
    pub fn start(tune: BoltTune, styles: [ArcTune; 2], seed: u32) -> Lightning {
        let mut rng = Mwc { lo: seed | 1, carry: 0x1234_5 };
        let mut bolts = [bolt_init(&tune, &mut rng); 10];
        let mut arcs = [[arc_init(&styles[0], &mut rng); 2]; 10];
        for i in 0..10 {
            bolts[i] = bolt_init(&tune, &mut rng);
            arcs[i] = [arc_init(&styles[0], &mut rng), arc_init(&styles[1], &mut rng)];
        }
        Lightning { bolts, arcs, tune, styles, rng }
    }

    /// One frame (sub_82375EB8): `rot` / `pos` the car's matrix, `bz` its bound's box maximum
    /// z, `hit` a ray from a to b against the car's body (world), `dt` 0 while paused.
    pub fn update(&mut self, rot: Quat, pos: Vec3, bz: f32, hit: &dyn Fn(Vec3, Vec3) -> Option<Vec3>, dt: f32) {
        // the bolt's frame B: the car's rows x bz, the third negated; + m_Offset in world axes
        let b = [rot * Vec3::X * bz, rot * Vec3::Y * bz, -(rot * Vec3::Z) * bz];
        let b3 = pos + self.tune.offset;
        for i in 0..10 {
            self.bolt_tick(i, b, b3, hit, dt);
            for k in 0..2 {
                arc_tick(&mut self.arcs[i][k], &self.styles[k], &mut self.rng, dt);
            }
        }
    }

    fn bolt_tick(&mut self, i: usize, b: [Vec3; 3], b3: Vec3, hit: &dyn Fn(Vec3, Vec3) -> Option<Vec3>, dt: f32) {
        // (sub_82749B48) theta += ArcSpeed dt, life -= dt; past EndTheta or out of life: anew
        {
            let bo = &mut self.bolts[i];
            bo.theta += bo.speed * dt;
            bo.life -= dt;
            if bo.theta > bo.end || bo.life < 0.0 {
                self.bolts[i] = bolt_init(&self.tune, &mut self.rng);
            }
        }
        let bo = &mut self.bolts[i];
        let rad = std::f32::consts::PI / 180.0;
        // S = diag(m_XSize, m_YSize, m_ZSize), its rows 0 / 1 turned by SampleRange
        // (sub_82227E98), then 0 / 2 by theta (sub_82214D90)
        let s = self.tune.size;
        let (mut s0, mut s1, mut s2) = (Vec3::new(s.x, 0.0, 0.0), Vec3::new(0.0, s.y, 0.0), Vec3::new(0.0, 0.0, s.z));
        let (c, sn) = ((bo.sample * rad).cos(), (bo.sample * rad).sin());
        let (n0, n1) = (s0 * c + s1 * sn, s1 * c - s0 * sn);
        s0 = n0;
        s1 = n1;
        let (c, sn) = ((bo.theta * rad).cos(), (bo.theta * rad).sin());
        let (n0, n2) = (s0 * c - s2 * sn, s2 * c + s0 * sn);
        s0 = n0;
        s2 = n2;
        // M2 = S'' x B (row vectors); the record keeps row 1 at +32, row 2 at +48, row 0 at +64
        let mul = |r: Vec3| b[0] * r.x + b[1] * r.y + b[2] * r.z;
        let (m0, m1, m2) = (mul(s0), mul(s1), mul(s2));
        bo.dir_a = m0.normalize_or_zero();
        bo.dir_b = m2.normalize_or_zero();
        // the five rays: angles -h, -0.1 h, 0, 0.5 h, h (h = Range / 2), d = (+48) cos - (+32) sin,
        // from B3 + d to B3 - d; the first hit on the body, else B3 + d
        let h = bo.range * 0.5;
        let angles = [-h, -h + 2.0 * h * 0.45, -h + 2.0 * h * 0.5, -h + 2.0 * h * 0.75, h];
        for (k, a) in angles.iter().enumerate() {
            let (c, sn) = ((a * rad).cos(), (a * rad).sin());
            let d = m2 * c - m1 * sn;
            let p = b3 + d;
            bo.points[k] = hit(p, b3 - d).unwrap_or(p);
        }
    }

    /// The strips to draw this frame: per bolt and style the camera-facing strip's pairs
    /// (left, right, colour, u) - sub_822EDD80 - `fade` the time left / ActiveTime.
    pub fn strips(&self, cam: Vec3, fade: f32) -> Vec<Vec<(Vec3, Vec3, [f32; 4], f32)>> {
        let mut out = Vec::new();
        for i in 0..10 {
            for k in 0..2 {
                if let Some(s) = arc_strip(&self.bolts[i], &self.arcs[i][k], &self.styles[k], cam, fade) {
                    out.push(s);
                }
            }
        }
        out
    }
}

fn arc_tick(a: &mut Arc, s: &ArcTune, rng: &mut Mwc, dt: f32) {
    // (sub_822EDAF0)
    a.growth += dt * a.growth_rate;
    a.flicker += dt * a.flicker_rate;
    a.tex += s.tex_anim_rate * dt;
    if a.growth > 1.0 {
        a.growth = 0.0;
        let _g = rng.range(s.arc_growth);
        a.growth_rate = 1.0 / rng.range(s.growth_speed);
    }
    if a.flicker > 1.0 {
        a.flicker = 0.0;
        a.flicker_rate = 1.0 / rng.range(s.flicker_speed);
        let _f = rng.range(s.flux_tangent);
        a.seed = rng.next() & 0x7FFF_FFFF;
    }
    if dt != 0.0 {
        // one number for all four channels
        let u = rng.unit();
        for c in 0..4 {
            a.color[c] = s.color_min[c] + (s.color_max[c] - s.color_min[c]) * u;
        }
    }
}

/// de Casteljau (sub_822ED750)
pub fn bezier(p: &[Vec3], t: f32) -> Vec3 {
    let mut q: Vec<Vec3> = p.to_vec();
    let n = q.len();
    for r in 1..n {
        for i in 0..n - r {
            q[i] = q[i] * (1.0 - t) + q[i + 1] * t;
        }
    }
    q.first().copied().unwrap_or(Vec3::ZERO)
}

/// One arc (sub_822EDD80): nothing for a Thickness under 0.01; `n` = Iterations points - the
/// first and last the curve's ends, those between at i / (n - 1) along the Bezier curve of the
/// five control points, moved by dir_a x m_ArcGrowth max x the growth clock and jittered with
/// the arc's own seed: each axis of dir_b x U(+-FluxTangent max x flicker clock) and of dir_a x
/// U(+-FluxNormal max x flicker clock); then n + 1 pairs at j / n along the Bezier curve of
/// those, +- the side (the chord x the view, normalised) x Thickness; the end pairs in
/// EndColor, the others in the arc's colour, both alphas x `fade`; u = j / n - the texture's
/// offset, v 0 / 1.
pub fn arc_strip(b: &Bolt, a: &Arc, s: &ArcTune, cam: Vec3, fade: f32) -> Option<Vec<(Vec3, Vec3, [f32; 4], f32)>> {
    if s.thickness < 0.01 || s.iterations < 2 {
        return None;
    }
    let n = s.iterations;
    let first = b.points[0];
    let last = b.points[4];
    let chord = (last - first).normalize_or_zero();
    let view = (first - cam).normalize_or_zero();
    let side = chord.cross(view).normalize_or_zero();
    let mut rng = Mwc { lo: a.seed, carry: 0 };
    let growth = s.arc_growth[1] * a.growth;
    let ft = s.flux_tangent[1] * a.flicker;
    let fnn = s.flux_normal[1] * a.flicker;
    let mut pts = vec![Vec3::ZERO; n];
    pts[0] = first;
    pts[n - 1] = last;
    for i in 1..n - 1 {
        let base = bezier(&b.points, i as f32 / (n - 1) as f32) + b.dir_a * growth;
        let mut u = || 2.0 * rng.unit() - 1.0;
        let t = Vec3::new(u() * ft * b.dir_b.x, u() * ft * b.dir_b.y, u() * ft * b.dir_b.z);
        let nn = Vec3::new(u() * fnn * b.dir_a.x, u() * fnn * b.dir_a.y, u() * fnn * b.dir_a.z);
        pts[i] = base + t + nn;
    }
    let mid = [a.color[0], a.color[1], a.color[2], a.color[3] * fade];
    let end = [s.end_color[0], s.end_color[1], s.end_color[2], s.end_color[3] * fade];
    let mut out = Vec::with_capacity(n + 1);
    for j in 0..=n {
        let t = j as f32 / n as f32;
        let p = bezier(&pts, t);
        let col = if j == 0 || j == n { end } else { mid };
        out.push((p - side * s.thickness, p + side * s.thickness, col, t - a.tex));
    }
    Some(out)
}

/// The tune file's values: the manager's (`m_*`) and a type's ma_ / mi_ (`section` the type's
/// name in the file).
pub fn load_tune(v: &std::collections::HashMap<String, Vec<f32>>, section: &str) -> (BoltTune, [ArcTune; 2]) {
    let f2 = |k: &str, d: [f32; 2]| v.get(k).filter(|x| x.len() >= 2).map_or(d, |x| [x[0], x[1]]);
    let f1 = |k: &str, d: f32| v.get(k).and_then(|x| x.first().copied()).unwrap_or(d);
    let off = v.get("m_Offset").filter(|x| x.len() >= 3).map_or(Vec3::new(0.0, 0.3725, 0.0), |x| Vec3::new(x[0], x[1], x[2]));
    let bt = BoltTune {
        size: Vec3::new(f1("m_XSize", 0.75), f1("m_YSize", 0.675), f1("m_ZSize", 1.4)),
        life: f2("m_LifeMinMax", [0.0, 10.0]),
        range: f2("m_RangeMinMax", [30.0, 60.0]),
        arc_speed: f2("m_ArcSpeedMinMax", [0.0, 80.0]),
        start_theta: f2("m_StartThetaMinMax", [0.0, 90.0]),
        end_theta: f2("m_EndThetaMinMax", [180.0, 180.0]),
        sample_range: f2("m_SampleRangeMinMax", [0.0, 180.0]),
        offset: off,
    };
    let arc = |p: &str| {
        let k = |n: &str| format!("{section}.{p}_{n}");
        let c4 = |n: &str| v.get(&k(n)).filter(|x| x.len() >= 4).map_or([0.0; 4], |x| [x[0], x[1], x[2], x[3]]);
        let r = |n: &str| v.get(&k(n)).filter(|x| x.len() >= 2).map_or([0.0; 2], |x| [x[0], x[1]]);
        let o = |n: &str| v.get(&k(n)).and_then(|x| x.first().copied()).unwrap_or(0.0);
        ArcTune {
            color_min: c4("ColorMin"),
            color_max: c4("ColorMax"),
            end_color: c4("EndColor"),
            arc_growth: r("ArcGrowthMinMax"),
            growth_speed: r("GrowthSpeedMinMax"),
            flux_tangent: r("FluxTangentRangeMinMax"),
            flux_normal: r("FluxNormalRangeMinMax"),
            flicker_speed: r("FlickerSpeedMinMax"),
            thickness: o("Thickness"),
            tex_anim_rate: o("TexAnimRate"),
            iterations: o("Iterations").max(0.0) as usize,
        }
    };
    (bt, [arc("ma"), arc("mi")])
}

/// Ray a -> b against a hull (car space triangles), the nearest hit in world space.
pub fn ray_hull(rot: Quat, pos: Vec3, verts: &[Vec3], tris: &[[u16; 3]], a: Vec3, b: Vec3) -> Option<Vec3> {
    let inv = rot.inverse();
    let (la, lb) = (inv * (a - pos), inv * (b - pos));
    let d = lb - la;
    let mut best: Option<f32> = None;
    for t in tris {
        let (p0, p1, p2) = (verts[t[0] as usize], verts[t[1] as usize], verts[t[2] as usize]);
        let (e1, e2) = (p1 - p0, p2 - p0);
        let h = d.cross(e2);
        let det = e1.dot(h);
        if det.abs() < 1e-9 {
            continue;
        }
        let f = 1.0 / det;
        let s = la - p0;
        let u = f * s.dot(h);
        if !(0.0..=1.0).contains(&u) {
            continue;
        }
        let q = s.cross(e1);
        let v = f * d.dot(q);
        if v < 0.0 || u + v > 1.0 {
            continue;
        }
        let tt = f * e2.dot(q);
        if (0.0..=1.0).contains(&tt) && best.map_or(true, |x| tt < x) {
            best = Some(tt);
        }
    }
    best.map(|t| pos + rot * (la + d * t))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The arc style's constructor run offline (tools/powerups_small_reference.py, table
    /// tests/powerups_small_reference.txt): the rates, the seed and the generator's state after, bit for bit.
    /// Evidence: docs/reference/powerups/verify_small_evidence.md section 2.
    #[test]
    fn arc_init_matches_the_original() {
        const REF: &str = include_str!("../tests/powerups_small_reference.txt");
        let mut n = 0;
        for line in REF.lines().filter(|l| l.starts_with("ARC ")) {
            let w: Vec<&str> = line.split_whitespace().collect();
            let dec = |s: &str| s[1..].parse::<u32>().unwrap();
            let v: Vec<f32> = w[3..11].iter().map(|s| f32::from_bits(dec(s))).collect();
            let tune = ArcTune {
                color_min: [0.0; 4],
                color_max: [0.0; 4],
                end_color: [0.0; 4],
                arc_growth: [v[0], v[1]],
                growth_speed: [v[2], v[3]],
                flux_tangent: [v[4], v[5]],
                flux_normal: [0.0; 2],
                flicker_speed: [v[6], v[7]],
                thickness: 0.0,
                tex_anim_rate: 0.0,
                iterations: 0,
            };
            let mut rng = Mwc { lo: dec(w[1]), carry: dec(w[2]) };
            let a = arc_init(&tune, &mut rng);
            assert_eq!(a.growth_rate.to_bits(), dec(w[12]), "growth rate (+20) of {line}");
            assert_eq!(a.flicker_rate.to_bits(), dec(w[14]), "flicker rate (+28) of {line}");
            assert_eq!(a.seed as u64, w[15][1..].parse::<u64>().unwrap(), "seed of {line}");
            assert_eq!((rng.lo, rng.carry), (dec(w[18]), dec(w[19])), "generator after {line}");
            n += 1;
        }
        assert_eq!(n, 1200);
    }

    #[test]
    fn bezier_and_random_numbers() {
        let p = [Vec3::ZERO, Vec3::new(1.0, 2.0, 0.0), Vec3::new(2.0, 0.0, 0.0)];
        assert_eq!(bezier(&p, 0.0), Vec3::ZERO);
        assert_eq!(bezier(&p, 1.0), Vec3::new(2.0, 0.0, 0.0));
        assert!((bezier(&p, 0.5) - Vec3::new(1.0, 1.0, 0.0)).length() < 1e-6);
        let mut r = Mwc { lo: 1, carry: 0 };
        assert_eq!(r.next(), 0x5CDC_FAA7);
        for _ in 0..1000 {
            let u = r.unit();
            assert!((0.0..1.0).contains(&u));
        }
    }

    #[test]
    fn bolts_attach_to_the_body() {
        let text = "type: a\nmcPowerUpMgr {\n  m_Offset 0 0.3725 0\n  Stop {\n    ma_Thickness 0.1125\n    ma_Iterations 10\n    ma_ColorMin 1 0 0 0.3\n    ma_ColorMax 1 0 0 0.04\n    ma_EndColor 1 0 0 0\n    ma_FluxNormalRangeMinMax 0 0.29\n    ma_GrowthSpeedMinMax 0.4 0.6\n    ma_FlickerSpeedMinMax 0.1 0.2\n  }\n}\n";
        let v = crate::powerups::parse_tune(text);
        let (bt, st) = load_tune(&v, "Stop");
        assert_eq!(st[0].iterations, 10);
        assert!((bt.offset.y - 0.3725).abs() < 1e-6);
        // a unit box as the body
        let verts: Vec<Vec3> = (0..8).map(|i| Vec3::new(if i & 1 == 0 { -1.0 } else { 1.0 }, if i & 2 == 0 { 0.0 } else { 1.4 }, if i & 4 == 0 { -2.2 } else { 2.2 })).collect();
        let tris: Vec<[u16; 3]> = vec![[0, 1, 3], [0, 3, 2], [4, 6, 7], [4, 7, 5], [0, 2, 6], [0, 6, 4], [1, 5, 7], [1, 7, 3], [2, 3, 7], [2, 7, 6], [0, 4, 5], [0, 5, 1]];
        let mut l = Lightning::start(bt, st, 7);
        let hit = |a: Vec3, b: Vec3| ray_hull(Quat::IDENTITY, Vec3::ZERO, &verts, &tris, a, b);
        l.update(Quat::IDENTITY, Vec3::ZERO, 2.2, &hit, 1.0 / 60.0);
        let mut on = 0;
        for b in &l.bolts {
            for p in &b.points {
                let q = *p;
                if (q.x.abs() - 1.0).abs() < 1e-3 || q.y.abs() < 1e-3 || (q.y - 1.4).abs() < 1e-3 || (q.z.abs() - 2.2).abs() < 1e-3 {
                    on += 1;
                }
            }
        }
        assert!(on >= 40, "{on} of 50 control points on the box");
        let s = l.strips(Vec3::new(0.0, 3.0, 10.0), 1.0);
        assert_eq!(s.len(), 10);
        assert_eq!(s[0].len(), 11);
        assert_eq!(s[0][0].2[3], 0.0);
    }
}
