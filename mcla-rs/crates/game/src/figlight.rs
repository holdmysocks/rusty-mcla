//! The walkers' four lights at night, as the original picks them.
//!
//! The walkers' night program (characters' VSPedNight + PSMultiLight) knows no lamp grid: its
//! light is diffuse x (gLightAmbient + four lanes of gMultiLightDynamicLightData), a lane giving
//! colour x sat(strength / d^2) x sat(cone) x sat(n.l). Their render routine (F-68474, the
//! flag byte 0x82830641 = 1) fills the lanes with `F-44222(mgr, p, walker + 64, 1)`:
//! * each walker keeps four light records that EASE towards the four lamps of the lamp-grid cell
//!   under it (lane k -> the cell's k-th lamp, an empty slot -> table lamp 0, which has no light):
//!   every call the direction from the walker to the record turns 2.5 % of the way, distance,
//!   colour, direction, strength and cone go 6 % of the way (F-79205);
//! * then the dynamic list (the player's two head lamps, the key light) is laid over lanes 3, 2,
//!   1 (and the fourth record, F-44222): a light takes the next lane when its score strictly beats the best so far - a head lamp
//!   sat(500 / d^2) x its cone at the walker, the key light (list entry 2) the luminance of its
//!   colour (0.3 r + 0.59 g + 0.11 b). Lane 0 stays a grid lamp.
//! Decoded 2026-10-20, docs/LIGHTING.md "The walkers' lights at night"; the rule reproduces the
//! dynamic lanes of all 52 walkers recorded (work/lighting2/picker).

use crate::render::{LampRec, LampTables};
use glam::{Quat, Vec3};

/// The lights the walkers' picker lays over the grid lamps this frame.
#[derive(Clone, Copy, Default)]
pub struct Dynamic {
    /// the player's two head lamps (strength 500, the cone sat(4.4696 cos - 2.872))
    pub heads: [Option<LampRec>; 2],
    /// the key light: colour and the direction towards it (100 km off)
    pub key: Option<(Vec3, Vec3)>,
    /// the list's fourth record: the siren's or the checkpoint's light, all round
    pub rec3: Option<LampRec>,
}

/// One walker's four eased records and its eased ambient (the original's walker + 64, + 384).
#[derive(Clone, Copy)]
pub struct FigureLight {
    cur: [LampRec; 4],
    amb: Vec3,
}

fn target(t: &LampTables, p: Vec3) -> [LampRec; 4] {
    t.cell(p).map(|i| t.recs.get(i as usize).copied().unwrap_or_default())
}

/// F-27981: `from` turned towards `to` by `frac` of the angle between them (to them at
/// once when they stand at right angles: |cos| < 0.001).
pub fn turn(from: Vec3, to: Vec3, frac: f32) -> Vec3 {
    let c = from.dot(to);
    if c.abs() < 0.001 {
        return to;
    }
    let axis = from.cross(to);
    if axis.length_squared() < 1e-12 {
        return from;
    }
    (Quat::from_axis_angle(axis.normalize(), c.clamp(-1.0, 1.0).acos() * frac) * from).normalize_or(to)
}

/// The score of a light at `p` (F-60154): sat(strength / d^2) x its cone.
pub fn score(l: &LampRec, p: Vec3) -> f32 {
    light_score(true, l.pos, -l.back, l.strength, l.cone[0], l.cone[1], p)
}

/// F-60154 step by step: 1 / d by the estimate and two Newton steps in double, 0 at d = 0 (and for a light that is
/// off), sat(strength / d^2) x sat(dot(dir, (p - pos) / d) x scale + bias), the fused steps as the original's.
/// Checked against the original's code in `render/lightref.rs`.
pub fn light_score(lit: bool, pos: Vec3, dir: Vec3, strength: f32, scale: f32, bias: f32, p: Vec3) -> f32 {
    if !lit {
        return 0.0;
    }
    let fma = |a: f32, b: f32, c: f32| (a as f64).mul_add(b as f64, c as f64) as f32;
    let sat = |x: f32| if x >= 0.0 { x.min(1.0) } else { 0.0 };
    let v = p - pos;
    let d2 = fma(v.y, v.y, fma(v.z, v.z, v.x * v.x)) as f64;
    let mut y = (1.0 / d2.sqrt()) as f32 as f64;
    for _ in 0..2 {
        let e = -(y * y).mul_add(d2, -1.0);
        y = e.mul_add(y * 0.5, y);
    }
    let r = if -d2 >= 0.0 { 0.0 } else { y as f32 };
    let w = v * r;
    let c = fma(fma(dir.y, w.y, fma(dir.z, w.z, dir.x * w.x)), scale, bias);
    sat(strength * r * r) * sat(c)
}

/// A lane as the figure's object uniform carries it: position and strength, colour, the axis
/// (the direction back to the light x the cone scale) and the cone bias.
fn lane(l: &LampRec) -> [[f32; 4]; 3] {
    let a = l.back * l.cone[0];
    [[l.pos.x, l.pos.y, l.pos.z, l.strength], [l.color.x, l.color.y, l.color.z, 0.0], [a.x, a.y, a.z, l.cone[1]]]
}

impl FigureLight {
    /// A walker first lit: its records and ambient start at their targets. (The original's state is zeroed once when
    /// its 256 walkers are built at load - F-37241 -> the light context's ctor - and then carries over from the walker
    /// object's last life; the port has no such pool: approximate.)
    pub fn new(t: &LampTables, p: Vec3, amb: Vec3) -> FigureLight {
        FigureLight { cur: target(t, p), amb }
    }

    /// F-79205's last step: the ambient eased 3 % (f32 0.03) a call towards `target` (F-44222's: the fill map at the
    /// walker, map / 255 x 0.25 x m_NFLFade, plus the scene ambient); `frames` as in `step`.
    pub fn step_amb(&mut self, target: Vec3, frames: f32) {
        let k = if frames == 1.0 { 0.03 } else { 1.0 - 0.97f32.powf(frames) };
        self.amb += (target - self.amb) * k;
    }

    /// The ambient as the light context packs it for the draw (F-59032): each channel clamped to 0..1, x 255, truncated.
    pub fn amb_bytes(&self) -> [u8; 3] {
        self.amb.to_array().map(|v| (v.clamp(0.0, 1.0) * 255.0) as u8)
    }

    /// One call of the original's picker at the walker's point `p`; `frames` = 30 Hz frames
    /// since the last (the original runs it once a drawn frame).
    pub fn step(&mut self, t: &LampTables, p: Vec3, frames: f32) {
        let (k6, k25) = (1.0 - 0.94f32.powf(frames), 1.0 - 0.975f32.powf(frames));
        for (c, g) in self.cur.iter_mut().zip(target(t, p)) {
            let (vc, vt) = (c.pos - p, g.pos - p);
            let (dc, dt) = (vc.length(), vt.length());
            let ut = if dt > 1e-6 { vt / dt } else { Vec3::Y };
            let uc = if dc > 1e-6 { turn(vc / dc, ut, k25) } else { ut };
            c.pos = p + uc * (dc + (dt - dc) * k6);
            c.color = c.color.lerp(g.color, k6);
            c.back = c.back.lerp(g.back, k6);
            c.strength += (g.strength - c.strength) * k6;
            // (the original eases the cone's angle and penumbra and rebuilds scale and bias from
            // them; here scale and bias themselves - the same at both ends)
            c.cone = [c.cone[0] + (g.cone[0] - c.cone[0]) * k6, c.cone[1] + (g.cone[1] - c.cone[1]) * k6];
        }
    }

    /// The four lanes at `p`: the eased records, the dynamic lights laid over lanes 3, 2, 1.
    pub fn lanes(&self, p: Vec3, d: &Dynamic) -> [[f32; 4]; 12] {
        let mut lanes = self.cur;
        let mut best = 0.0f32;
        let mut next = 3usize;
        // (the key light's record stands 100 km from the world's origin along its direction, not from the walker: every
        // walker of work/checkpoints2/cap/col4.bin, 01:06, has it at (69151.3, -72225.4, -1260.7) in its lane 2)
        let key = d.key.map(|(col, dir)| LampRec { pos: dir.normalize_or(Vec3::Y) * 1.0e5, color: col, strength: 1.0e10, back: Vec3::ZERO, cone: [0.0, 1.0] });
        for (j, l) in [d.heads[0], d.heads[1], key, d.rec3].into_iter().enumerate() {
            let Some(l) = l else { continue };
            let s = if j == 2 { 0.3 * l.color.x + 0.59 * l.color.y + 0.11 * l.color.z } else { score(&l, p) };
            if s > best {
                best = s;
                if next > 0 {
                    lanes[next] = l;
                    next -= 1;
                }
            }
        }
        let mut out = [[0.0; 4]; 12];
        for (k, l) in lanes.iter().enumerate() {
            out[3 * k..3 * k + 3].copy_from_slice(&lane(l));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tables() -> LampTables {
        let lamp = |x: f32, c: f32| LampRec { pos: Vec3::new(x, 25.0, 0.0), color: Vec3::splat(c), strength: 40.0, back: Vec3::ZERO, cone: [0.0, 1.0] };
        LampTables { cells: vec![[1, 2, 0, 0]; (crate::render::LAMP_GRID.0 * crate::render::LAMP_GRID.1) as usize], recs: vec![LampRec::default(), lamp(5.0, 1.0), lamp(-5.0, 0.5)] }
    }

    /// The recorded walker of f4 draw 1114: a head lamp 146 m off loses lane 2 to the key light
    /// (0.038 grey) and the far head lamp keeps lane 3 - the running best, list order.
    #[test]
    fn dynamic_lanes_by_running_best() {
        let t = tables();
        let f = FigureLight::new(&t, Vec3::ZERO, Vec3::ZERO);
        let head = |z: f32| LampRec { pos: Vec3::new(-140.8, 0.0, z), color: Vec3::new(2.1647, 2.0565, 1.858), strength: 500.0, back: Vec3::new(-0.724, 0.690, 0.004), cone: [4.4696, -2.872] };
        // (146 m: sat(500 / d^2) = 0.023, cone 0.25 - a score of 0.006; head lamp 1 a little nearer
        // scores higher and takes lane 2, the key light's 0.038 beats both and takes lane 1)
        let d = Dynamic { heads: [Some(head(40.0)), Some(head(39.0))], key: Some((Vec3::splat(0.038), Vec3::Y)), rec3: None };
        let l = f.lanes(Vec3::ZERO, &d);
        assert_eq!(l[0][3], 40.0, "lane 0 stays the grid lamp");
        assert_eq!((l[9][3], l[6][3], l[3][3]), (500.0, 500.0, 1.0e10));
        // (the key light's place: 1e5 along its direction from the world's origin, wherever the walker stands)
        let l = f.lanes(Vec3::new(-430.15, 17.98, -651.22), &d);
        let k = (0..4).find(|k| l[3 * k][3] == 1.0e10).expect("the key light in a lane");
        assert_eq!((l[3 * k][0], l[3 * k][1], l[3 * k][2]), (0.0, 1.0e5, 0.0));
        // head lamp 1 outside its cone scores 0 and cannot beat head lamp 0: the key takes lane 2,
        // lane 1 keeps its grid lamp
        let d2 = Dynamic { heads: [Some(head(39.0)), Some(head(80.0))], key: Some((Vec3::splat(0.038), Vec3::Y)), rec3: None };
        let l2 = f.lanes(Vec3::ZERO, &d2);
        assert_eq!((l2[9][3], l2[6][3], l2[3][3]), (500.0, 1.0e10, 40.0));
        // the fourth record beats the key light's 0.038 only with a higher score; the lanes have run out after 3
        let siren = LampRec { pos: Vec3::new(0.0, 2.0, 5.0), color: Vec3::Z, strength: 200.0, back: Vec3::ZERO, cone: [0.0, 1.0] };
        let d3 = Dynamic { heads: [Some(head(39.0)), Some(head(80.0))], key: Some((Vec3::splat(0.038), Vec3::Y)), rec3: Some(siren) };
        let l3 = f.lanes(Vec3::ZERO, &d3);
        assert_eq!((l3[9][3], l3[6][3], l3[3][3]), (500.0, 1.0e10, 200.0));
    }

    /// The walkers of the original's title street at 23:00, 40 consecutive frames (testdata/walker_ambient_ref.txt): each
    /// walker's c26 bytes follow from one start value inside its first byte, eased 3 % a frame towards the scene ambient
    /// (9, 13, 15) / 255 plus the fill map at the walker's recorded place (m_NFLFade 1), packed by truncation.
    #[test]
    fn walkers_ambient_eases_as_recorded() {
        let dir = "D:/MCLA/extracted/cache";
        if !std::path::Path::new(dir).join("tune/city/sc").exists() {
            return;
        }
        crate::fillmap::init(dir);
        crate::fillmap::set_fade(1.0);
        let base = Vec3::new(9.0, 13.0, 15.0) / 255.0;
        let text = include_str!("../testdata/walker_ambient_ref.txt");
        let text = text.replace('\r', "");
        let walkers: Vec<Vec<(f32, f32, [u8; 3])>> = text
            .split("\n--\n")
            .skip(1)
            .map(|w| {
                w.lines()
                    .filter(|l| !l.trim().is_empty())
                    .map(|l| {
                        let v: Vec<f32> = l.split_whitespace().map(|x| x.parse().unwrap()).collect();
                        (v[0], v[1], [v[2] as u8, v[3] as u8, v[4] as u8])
                    })
                    .collect()
            })
            .collect();
        // series of a walker that stays in one cell of the map (one target) / of those reproduced; the same for the walkers
        // crossing a cell edge, where the original's sample point (F-58860: a bone or the instance matrix) and the draw's
        // place may lie on different sides
        let (mut steady, mut steady_fit, mut cross, mut cross_fit, mut changing) = (0, 0, 0, 0, 0);
        for w in &walkers {
            let tgt: Vec<Vec3> = w.iter().map(|&(x, z, _)| { let c = crate::fillmap::body_now(x, z); base + Vec3::new(c[0], c[1], c[2]) }).collect();
            let one_cell = tgt.iter().all(|t| *t == tgt[0]);
            for ch in 0..3 {
                changing += (w.iter().any(|f| f.2[ch] != w[0].2[ch])) as usize;
                let ok = (0..400).any(|i| {
                    let mut f = FigureLight { cur: [LampRec::default(); 4], amb: Vec3::splat((w[0].2[ch] as f32 + i as f32 / 400.0) / 255.0) };
                    w.iter().enumerate().all(|(n, &(_, _, b))| {
                        if n > 0 {
                            f.step_amb(tgt[n], 1.0);
                        }
                        f.amb_bytes()[ch] == b[ch]
                    })
                });
                if one_cell { steady += 1; steady_fit += ok as usize } else { cross += 1; cross_fit += ok as usize }
            }
        }
        eprintln!("walkers {} changing series {changing}; in one cell {steady_fit} of {steady}; crossing a cell edge {cross_fit} of {cross}", walkers.len());
        assert_eq!(walkers.len(), 51);
        assert_eq!(steady_fit, steady, "series in one cell not reproduced");
    }

    /// The records ease: 6 % of the distance and colour per 30 Hz call.
    #[test]
    fn records_ease_towards_the_cell() {
        let t = tables();
        let mut f = FigureLight { cur: [LampRec { pos: Vec3::new(10.0, 25.0, 0.0), color: Vec3::ZERO, strength: 0.0, back: Vec3::ZERO, cone: [0.0, 1.0] }; 4], amb: Vec3::ZERO };
        f.step(&t, Vec3::ZERO, 1.0);
        assert!((f.cur[0].color.x - 0.06).abs() < 1e-6);
        assert!((f.cur[0].strength - 2.4).abs() < 1e-4);
    }
}
