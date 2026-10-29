//! The tyres' particle effects of every car on the original's simulation (player, racers): the
//! "tire particles" table `$/tune/rmptfx/tireptx` (12 rows of 4 effects), chosen per wheel by the
//! ground's material (`ptxindex`, `ptxthreshold`), the slide, the burnout and the road's wetness,
//! with the effects' evolution values - `sub_82351A70` / `sub_82748C48` / `sub_82748910`.
//! Rule: `docs/reference/map/tireptx_rule.md`.

use crate::particles::Particles;
use crate::physics::collision::Surface;
use crate::vehicles::CarInstance;
use glam::{Mat3, Quat, Vec3};
use hashbrown::HashMap;
use mcla_formats::hash::joaat;
use std::path::Path;

/// One wheel's state (mcCar +1012, 144 bytes each).
#[derive(Clone, Copy, Default)]
struct State {
    /// the effect playing (S+0 / +52: joaat of its name), none when 0
    rule: u32,
    /// last skidAmount, rainy (S+32, +36), the smoothed donut (S+40), the linger count (S+48)
    skid: f32,
    rainy: f32,
    donut: f32,
    linger: i32,
    /// the matrix the instance follows (S+64: side, normal, backward, contact point)
    mtx: [Vec3; 4],
    tire_size: f32,
    speed: f32,
    fwd: bool,
}

/// What the original decides for a wheel each frame.
pub struct TyreFx {
    /// rows of (slide, drive, wet, burnout) effects; None = no rule ("none", or not in the list)
    table: Vec<[Option<u32>; 4]>,
    /// material -> ptxindex[0], [1] and ptxthreshold[0], [1] (material record +44 .. +52)
    mats: HashMap<u8, ([i16; 2], [f32; 2])>,
    /// per car (its index and model, to notice another car in the slot): four states
    cars: HashMap<usize, (usize, [State; 4])>,
    /// the road's wetness (0x8287E130), eased to the weather's at dt / 5 a frame (`sub_822E8BA8`)
    pub wet: f32,
}

/// The effect list's own names: the table's names are looked up as effect rules
/// (`sub_821c9790`); one that is not in MC4.r107 (`test_smile`) gives none.
fn rule(fx: &Particles, name: &str) -> Option<u32> {
    let h = joaat(name);
    (!name.eq_ignore_ascii_case("none") && fx.has_effect(h)).then_some(h)
}

impl TyreFx {
    pub fn load(data_dir: &Path, fx: &Particles) -> TyreFx {
        let txt = std::fs::read_to_string(data_dir.join("tune/rmptfx/0x09af926c")).unwrap_or_default();
        let mut table = Vec::new();
        for l in txt.lines() {
            let t: Vec<&str> = l.split_whitespace().collect();
            if t.len() >= 4 {
                table.push([rule(fx, t[0]), rule(fx, t[1]), rule(fx, t[2]), rule(fx, t[3])]);
            }
        }
        let mut mats = HashMap::new();
        let dir = data_dir.join("tune/physics/materials");
        for s in Surface::ALL {
            let name = s.material();
            let txt = [dir.join(format!("0x{:08x}", joaat(&format!("{name}.mtl")))), dir.join(format!("0x{:08x}", joaat(name)))].iter().find_map(|p| std::fs::read_to_string(p).ok());
            let Some(txt) = txt else { continue };
            let field = |k: &str| -> Vec<f32> { txt.lines().find_map(|l| l.trim().strip_prefix(k).map(|v| v.split_whitespace().filter_map(|x| x.parse().ok()).collect())).unwrap_or_default() };
            let (ix, th) = (field("ptxindex:"), field("ptxthreshold:"));
            let g = |v: &[f32], i: usize, d: f32| v.get(i).copied().unwrap_or(d);
            mats.insert(s as u8, ([g(&ix, 0, -1.0) as i16, g(&ix, 1, -1.0) as i16], [g(&th, 0, 0.0), g(&th, 1, 0.0)]));
        }
        log::info!("tyre effects: {} rows, {} materials", table.len(), mats.len());
        TyreFx { table, mats, cars: HashMap::new(), wet: 0.0 }
    }

    /// colN[idx]: none when (u32)idx >= the row count
    fn col(&self, c: usize, idx: i16) -> Option<u32> {
        self.table.get(idx as u16 as usize).and_then(|r| r[c])
    }

    /// One game frame (30 Hz, `frame` true) of every car's tyre effects, or between frames (`frame`
    /// false) the playing ones moved to where the wheels are now. `weather_wet`: the weather's
    /// wetness the road eases to; `dt` the frame's step; `cam` for the effects' cull distance.
    pub fn step(&mut self, fx: &mut Particles, cars: &[CarInstance], weather_wet: f32, dt: f32, frame: bool, cam: Vec3) {
        if frame {
            let k = dt / 5.0;
            self.wet = if (weather_wet - self.wet).abs() <= k { weather_wet } else { self.wet + k * (weather_wet - self.wet).signum() };
        }
        self.cars.retain(|i, (m, _)| cars.get(*i).is_some_and(|c| std::sync::Arc::as_ptr(&c.model) as usize == *m));
        for (ci, c) in cars.iter().enumerate() {
            let Some(vs) = c.state.veh.as_deref() else { continue };
            let Some(tune) = c.model.params.veh.as_deref() else { continue };
            if c.parked.is_some() || !c.placed {
                continue;
            }
            let model = std::sync::Arc::as_ptr(&c.model) as usize;
            let mut states = self.cars.remove(&ci).map(|(_, s)| s).unwrap_or_default();
            // (burnout charge chassis +580 or the tyres' melt +660 -> +24)
            let flag_a = vs.burn > 0.0 || vs.melt > 0.0;
            // the states in the original's order: the freetrain's wheels (+404), then the powertrain's
            // (+400), each list left before right (mirrored on even places); a bike: two per wheel
            let list: Vec<(usize, bool, bool)> = if c.model.bike {
                vec![(0, true, false), (0, false, false), (2, true, flag_a), (2, false, flag_a)]
            } else {
                let power: &[usize] = match vs.drive {
                    0 => &[2, 3],
                    1 | 4 => &[0, 1],
                    _ => &[0, 1, 2, 3],
                };
                let free: Vec<usize> = (0..4).filter(|w| !power.contains(w)).collect();
                let mut l: Vec<(usize, bool, bool)> = free.iter().enumerate().map(|(k, &w)| (w, k % 2 == 0, false)).collect();
                l.extend(power.iter().enumerate().map(|(k, &w)| (w, k % 2 == 0, flag_a)));
                l
            };
            for (si, &(wi, mirror, fa)) in list.iter().enumerate().take(4) {
                let key = 0x5459_5245_0000_0000 | ((ci as u64) << 4) | si as u64;
                let s = &mut states[si];
                let w = &vs.wheels[wi];
                // the matrix: side (negated mirrored), normal, backward (forward while the wheel turns backwards), point
                s.mtx = [if mirror { -w.frame[0] } else { w.frame[0] }, w.frame[1], if w.spin_rate < 0.0 { -w.frame[2] } else { w.frame[2] }, w.frame[3]];
                if frame {
                    let wt = if wi < 2 { &tune.t.wheel_front } else { &tune.t.wheel_back };
                    self.decide(s, w, wt, fa, vs.drive, c.state.vel, dt);
                }
                if s.rule != 0 {
                    let ((rot, flip), pos) = (mtx_rot(&s.mtx), s.mtx[3]);
                    let evo = [("skidAmount", s.skid), ("rainy", s.rainy), ("donut", s.donut.clamp(0.0, 1.0)), ("tireSize", s.tire_size), ("speed", s.speed), ("fwd", if s.fwd { 1.0 } else { 0.0 })];
                    fx.play_evo(key, s.rule, pos, rot, flip, Vec3::ZERO, pos.distance(cam), &evo);
                    if frame && c.is_player && std::env::var_os("MCLA_TYRE_LOG").is_some() {
                        log::info!("tyre: wheel {wi} state {si} rule {:08x} flip {flip} x.right {:.2} at {pos:.2?} slide {:.2} vlong {:.2} burn {:.2} drive {} donut {:.2}", s.rule, (rot * if flip { -Vec3::X } else { Vec3::X }).dot(c.state.right()), vs.wheels[wi].slide, vs.wheels[wi].velocity_long, vs.burn, vs.drive, s.donut);
                    }
                }
            }
            self.cars.insert(ci, (model, states));
        }
    }

    /// A car's reset (the model's full reset F-74884 and the restore's F-11744, through F-83592 per wheel state): the
    /// four states' effects stop at once, their particles gone (`Particles::kill`); a state that still wants its
    /// effect starts it anew on its next frame.
    pub fn kill(&mut self, fx: &mut Particles, ci: usize) {
        for si in 0..4 {
            fx.kill(0x5459_5245_0000_0000 | ((ci as u64) << 4) | si as u64);
        }
    }

    /// `sub_82748C48` for one wheel.
    fn decide(&self, s: &mut State, w: &crate::physics::vehsim::WheelSim, wt: &mcla_formats::rage::simtune::WheelTune, flag_a: bool, drive: u8, vel: Vec3, dt: f32) {
        // (a wheel without a material - in the air - leaves its effect as it is)
        let Some(m) = w.ground.and_then(|g| self.mats.get(&g)) else { return };
        let (ix, thr) = *m;
        // the suspension's compression over SuspensionLimit (sub_82564E28; 0 under rest)
        let cc = w.comp.clamp(-wt.travel_extent, wt.travel_limit);
        let comp = if cc < 0.0 || wt.suspension_limit == 0.0 { 0.0 } else { cc / wt.suspension_limit };
        let (slide, on, vlong) = (w.slide, w.record, w.velocity_long);
        let mut rule = None;
        if flag_a && vlong.abs() < 8.0 {
            rule = self.col(3, ix[0]);
            if rule.is_some() {
                s.linger = 10;
            }
        } else if self.wet >= 0.9 {
            if slide > 0.0 && on {
                rule = self.col(2, ix[0]);
                if rule.is_some() {
                    s.linger = 10;
                }
            }
        } else {
            if thr[1] > 0.0 {
                if comp > thr[1] && slide > 0.0 {
                    rule = self.col(1, ix[1]);
                    if rule.is_some() {
                        s.linger = 0;
                    }
                }
            } else if slide > 0.0 && on {
                rule = self.col(1, ix[1]);
                if rule.is_some() {
                    s.linger = 10;
                }
            }
            if flag_a || slide > thr[0] {
                rule = self.col(0, ix[0]);
                if rule.is_some() {
                    s.linger = 10;
                }
            }
        }
        let r = wt.radius;
        s.tire_size = if r <= 0.18 { 0.0 } else if r >= 0.64 { 1.0 } else { (r - 0.18) * 2.173_913_2 };
        s.fwd = drive == 1 || drive == 3;
        if !flag_a {
            s.donut = 1.1;
        }
        let v = vel.length();
        s.speed = if v <= 15.0 { 0.0 } else if v < 25.0 { (v - 15.0) / 10.0 } else { 1.0 };
        match rule {
            Some(rl) => {
                s.skid = ((slide - thr[0]) / (1.0 - thr[0])).clamp(0.0, 1.0);
                s.rainy = (vlong.abs() * 0.04).clamp(0.0, 1.0).mul_add((self.wet - 0.9) * 0.909_090_9, s.skid);
                if flag_a {
                    let tgt = ((w.velocity_lat * w.velocity_lat + vlong * vlong).sqrt() * 0.125).clamp(0.0, 1.0);
                    if s.donut > tgt {
                        s.donut = (s.donut - dt * 0.25).max(tgt);
                    } else if s.donut < tgt {
                        s.donut = tgt;
                    }
                }
                s.rule = rl;
            }
            None => {
                // (10 more frames with the last rule and values, then a soft stop - particles live out)
                if s.linger <= 0 {
                    s.rule = 0;
                } else if s.rule != 0 {
                    s.linger -= 1;
                }
            }
        }
    }
}

/// The instance's frame from the matrix's rows (x side, y normal, z backward): its turn, and
/// whether it is a mirror image (the side negated on the left wheels, the backward row on a wheel
/// turning backwards) - then the turn is the one of the frame with x negated back.
fn mtx_rot(m: &[Vec3; 4]) -> (Quat, bool) {
    let (x, y, z) = (m[0].normalize_or(Vec3::X), m[1].normalize_or(Vec3::Y), m[2].normalize_or(Vec3::Z));
    let flip = x.cross(y).dot(z) < 0.0;
    let x = if flip { -x } else { x };
    // (the contact's rows are not exactly square to each other: the nearest turn)
    let y = (y - x * x.dot(y)).normalize_or(Vec3::Y);
    let z = x.cross(y);
    (Quat::from_mat3(&Mat3::from_cols(x, y, z)).normalize(), flip)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::physics::vehsim::WheelSim;
    use mcla_formats::rage::simtune::WheelTune;

    /// a table of four rows whose effects are named by their row and column (row * 10 + column + 1)
    fn fx_table() -> TyreFx {
        let table = (0..4u32).map(|r| [Some(r * 10 + 1), Some(r * 10 + 2), Some(r * 10 + 3), Some(r * 10 + 4)]).collect();
        let mut mats = HashMap::new();
        // road (0, -1, 0.9, 0.8); grass (2, 2, 0.2, 0); dirt (3, 3, 0.1, 0)
        mats.insert(Surface::Road as u8, ([0, -1], [0.9, 0.8]));
        mats.insert(Surface::Grass as u8, ([2, 2], [0.2, 0.0]));
        mats.insert(Surface::Dirt as u8, ([3, 3], [0.1, 0.0]));
        TyreFx { table, mats, cars: HashMap::new(), wet: 0.0 }
    }

    fn wheel(surface: Surface, slide: f32, vlong: f32) -> WheelSim {
        WheelSim { ground: Some(surface as u8), slide, velocity_long: vlong, record: true, ..Default::default() }
    }

    fn tune() -> WheelTune {
        WheelTune { radius: 0.34, travel_limit: 0.2, travel_extent: 0.2, suspension_limit: 0.1, ..Default::default() }
    }

    #[test]
    fn columns_by_the_original_rule() {
        let t = fx_table();
        let mut s = State::default();
        // a burnout (charge > 0) under 8 m/s forward: the burnout column of the material's first row
        t.decide(&mut s, &wheel(Surface::Road, 0.3, 2.0), &tune(), true, 0, Vec3::ZERO, 1.0 / 30.0);
        assert_eq!((s.rule, s.linger), (4, 10));
        // the same at 9 m/s: the slide column (flag A replaces the drive column)
        let mut s = State::default();
        t.decide(&mut s, &wheel(Surface::Road, 0.3, 9.0), &tune(), true, 0, Vec3::ZERO, 1.0 / 30.0);
        assert_eq!(s.rule, 1);
        // the road without a burnout: thr1 0.8 asks the compression; slide under thr0 0.9 - nothing
        let mut s = State::default();
        t.decide(&mut s, &wheel(Surface::Road, 0.5, 9.0), &tune(), false, 0, Vec3::ZERO, 1.0 / 30.0);
        assert_eq!(s.rule, 0);
        // over thr0: the slide column, skidAmount (0.95 - 0.9) / 0.1
        t.decide(&mut s, &wheel(Surface::Road, 0.95, 9.0), &tune(), false, 0, Vec3::ZERO, 1.0 / 30.0);
        assert_eq!(s.rule, 1);
        assert!((s.skid - 0.5).abs() < 1e-5, "{}", s.skid);
        // grass with any slide on the ground: the drive column of its second index (thr1 0)
        let mut s = State::default();
        t.decide(&mut s, &wheel(Surface::Grass, 0.1, 9.0), &tune(), false, 0, Vec3::ZERO, 1.0 / 30.0);
        assert_eq!((s.rule, s.linger), (22, 10));
        // wet road (>= 0.9): the wet column while sliding on the ground
        let mut w = fx_table();
        w.wet = 1.0;
        let mut s = State::default();
        w.decide(&mut s, &wheel(Surface::Dirt, 0.1, 10.0), &tune(), false, 0, Vec3::new(30.0, 0.0, 0.0), 1.0 / 30.0);
        assert_eq!(s.rule, 33);
        // rainy = skid + clamp(0.04 |v|) x (wet - 0.9) / 0.11; speed (30 - 15) / 10 clamped
        assert!((s.rainy - (0.0 + 0.4 * 0.1 * 0.909_090_9)).abs() < 1e-5, "{}", s.rainy);
        assert_eq!(s.speed, 1.0);
    }

    #[test]
    fn linger_ten_frames_then_off() {
        let t = fx_table();
        let mut s = State::default();
        t.decide(&mut s, &wheel(Surface::Grass, 0.5, 5.0), &tune(), false, 0, Vec3::ZERO, 1.0 / 30.0);
        assert_eq!(s.rule, 21);
        let mut on = 0;
        for _ in 0..20 {
            t.decide(&mut s, &wheel(Surface::Grass, 0.0, 5.0), &tune(), false, 0, Vec3::ZERO, 1.0 / 30.0);
            if s.rule != 0 {
                on += 1;
            }
        }
        assert_eq!(on, 10);
    }

    #[test]
    fn evolution_values() {
        let t = fx_table();
        let mut s = State::default();
        // tireSize (0.34 - 0.18) / 0.46; fwd for layouts 1 and 3; donut 1.1 without a burnout
        t.decide(&mut s, &wheel(Surface::Road, 0.95, 9.0), &tune(), false, 3, Vec3::ZERO, 1.0 / 30.0);
        assert!((s.tire_size - 0.16 * 2.173_913_2).abs() < 1e-6);
        assert!(s.fwd);
        assert_eq!(s.donut, 1.1);
        // a burnout: the donut goes to |contact velocity| / 8 at once when below it, down at 0.25 a second
        let mut w = wheel(Surface::Road, 0.3, 4.0);
        w.velocity_lat = 3.0;
        t.decide(&mut s, &w, &tune(), true, 0, Vec3::ZERO, 1.0 / 30.0);
        assert!((s.donut - (1.1 - 0.25 / 30.0)).abs() < 1e-6, "{}", s.donut);
        s.donut = 0.2;
        t.decide(&mut s, &w, &tune(), true, 0, Vec3::ZERO, 1.0 / 30.0);
        assert!((s.donut - 5.0 * 0.125).abs() < 1e-6);
    }

    /// The disc's table and materials, and how the tyre smoke is drawn (`tireptx_rule.md`).
    #[test]
    #[ignore]
    fn table_and_programs_from_the_disc() {
        const DATA: &str = "D:/MCLA/extracted/cache";
        let txt = std::fs::read_to_string(Path::new(DATA).join("tune/rmptfx/0x09af926c")).unwrap();
        let rows: Vec<Vec<&str>> = txt.lines().map(|l| l.split_whitespace().collect::<Vec<_>>()).filter(|r| r.len() >= 4).collect();
        assert_eq!(rows.len(), 12);
        assert_eq!(rows[0], ["wheel_s_asphalt", "wheel_s_asphalt", "wheel_s_water_rooster", "wheel_b_asphalt"]);
        let res = crate::assets::load_resource_file(&Path::new(DATA).join("resources/rmptfx/fxlists/MC4.r107")).unwrap();
        let fx = mcla_formats::rage::ptfx::FxList::parse(&res).unwrap();
        // every name but the ice row's test_smile and row 11's "none" is an effect rule of the list
        let missing: Vec<&str> = rows.iter().flatten().copied().filter(|n| *n != "none" && fx.effect(n).is_none()).collect();
        assert!(missing.iter().all(|n| *n == "test_smile"), "{missing:?}");
        // the burnout smoke: smoke_cloud_tyres 7 x 7, draw_SoftPart_TexBlend_VertLit_Rescale (mode 6), alpha-blended
        let r = fx.particle_rules.iter().find(|r| r.name == "wheel_b_smoke_down_up").unwrap();
        let sp = r.sprite.as_ref().unwrap();
        assert_eq!((sp.texture.as_str(), sp.tiles_x, sp.tiles_y, r.render.blend_set), ("smoke_cloud_tyres", 7, 7, 0));
        assert_eq!(crate::particles::sprite_mode(sp, r.render.blend_set), 6.0);
        // the slide smoke spawns by distance only (SpawnRateKF 0)
        let e = fx.emit_rules.iter().find(|e| e.name == "wheel_s_asphalt_smoke").unwrap();
        assert_eq!(e.spawn_rate.eval(0.5)[..2], [0.0, 0.0]);
        // road: rows 0 / none, thresholds 0.9 / 0.8
        let t = std::fs::read_to_string(Path::new(DATA).join(format!("tune/physics/materials/0x{:08x}", joaat("road.mtl")))).unwrap();
        assert!(t.contains("ptxindex: 0  -1") && t.contains("ptxthreshold: 0.89999998  0.80000001"), "{t}");
    }

    #[test]
    fn mirrored_frames() {
        let m = [Vec3::X, Vec3::Y, Vec3::Z, Vec3::ZERO];
        assert!(!mtx_rot(&m).1);
        let (q, f) = mtx_rot(&[-Vec3::X, Vec3::Y, Vec3::Z, Vec3::ZERO]);
        assert!(f);
        assert!((q * Vec3::X - Vec3::X).length() < 1e-5);
    }
}
