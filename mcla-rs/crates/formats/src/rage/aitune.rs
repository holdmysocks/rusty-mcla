//! The computer drivers' picture of a car (`aivVehicleTuning`): two unnamed components
//! of the tuning resource (type 50), stock and fully upgraded, each 2195 floats that the
//! developers' calibration runs measured on the real simulation:
//!
//! ```text
//! [0] MaxSpeed  [1] MinCurvature  [2] MaxCurvature  [3] MaxTotalAccel  [4] MaxBrakeDecel
//! [5..8] DragConstant1..3   [8..12] derived scales   [12..19] IdleSpeed per gear
//! [19..]   MaxEngineAccelByGear  7 rows x 128 (row 0: the best gear), against speed
//! [915..]  MaxEngineDecelByGear  7 rows x 128
//! [1939..] MaxSpeedForHighCurvature 64   [2003..] MaxSpeedForLowCurvature 64
//! [2067..] SteerScalar 128, against speed
//! ```
//!
//! Speed tables are indexed by `speed * 127 / MaxSpeed`; the low curvature table covers
//! 0 .. MaxCurvature/10 in 64 steps, the high one MaxCurvature/10 .. MaxCurvature in 63.
use super::Rd;
use crate::rsc::Resource;

const N: usize = 2195;

#[derive(Debug, Clone)]
pub struct AiTune {
    pub max_speed: f32,
    pub min_curvature: f32,
    pub max_curvature: f32,
    pub max_total_accel: f32,
    pub max_brake_decel: f32,
    pub idle_speed: [f32; 7],
    /// full-throttle acceleration in the best gear against speed
    pub accel: Vec<f32>,
    /// engine braking of each gear against speed
    pub decel: Vec<Vec<f32>>,
    pub speed_high_curv: Vec<f32>,
    pub speed_low_curv: Vec<f32>,
    /// steering input per unit of curvature, against speed
    pub steer_scalar: Vec<f32>,
    /// [5..8] DragConstant1..3
    pub drag: [f32; 3],
    /// the whole MaxEngineAccelByGear block as the class reads it: rows of 128 from [19], row g - 1
    /// for gear g (the original's numbering, 2 = first: sub_82549B78 reads [147 + 128 (g - 2) + i]);
    /// row 0 is the best of rows 1..7 (sub_82549770)
    pub accel_rows: Vec<Vec<f32>>,
}

fn sample(table: &[f32], x: f32) -> f32 {
    if table.is_empty() {
        return 0.0;
    }
    let x = x.clamp(0.0, table.len() as f32 - 1.0);
    let i = (x as usize).min(table.len() - 1);
    let j = (i + 1).min(table.len() - 1);
    table[i] + (table[j] - table[i]) * (x - i as f32)
}

impl AiTune {
    fn read(rd: &Rd, c: usize) -> Option<AiTune> {
        let f = |i: usize| rd.f32(c + i * 4);
        if std::env::var("MCLA_AITUNE_DUMP").is_ok() {
            eprintln!("aitune header {:?}", (0..19).map(|i| f(i)).collect::<Vec<_>>());
        }
        let t = AiTune {
            max_speed: f(0),
            min_curvature: f(1),
            max_curvature: f(2),
            max_total_accel: f(3),
            max_brake_decel: f(4),
            idle_speed: std::array::from_fn(|i| f(12 + i)),
            accel: (0..128).map(|i| f(19 + i)).collect(),
            decel: (0..7).map(|g| (0..128).map(|i| f(915 + g * 128 + i)).collect()).collect(),
            speed_high_curv: (0..64).map(|i| f(1939 + i)).collect(),
            speed_low_curv: (0..64).map(|i| f(2003 + i)).collect(),
            steer_scalar: (0..128).map(|i| f(2067 + i)).collect(),
            drag: [f(5), f(6), f(7)],
            accel_rows: (0..8).map(|g| (0..128).map(|i| f(19 + g * 128 + i)).collect()).collect(),
        };
        let sane = t.max_speed > 5.0 && t.max_speed < 200.0 && t.max_curvature > 0.01 && t.max_curvature < 2.0 && t.max_total_accel > 1.0 && t.speed_high_curv.iter().all(|v| v.is_finite() && *v >= 0.0);
        sane.then_some(t)
    }

    /// The fastest the car holds a bend of this curvature (1/m).
    pub fn speed_for_curvature(&self, k: f32) -> f32 {
        let k = k.abs();
        let knee = self.max_curvature * 0.1;
        if k < knee {
            sample(&self.speed_low_curv, k * 64.0 / knee)
        } else {
            sample(&self.speed_high_curv, (k - knee) * 63.0 / (self.max_curvature - knee).max(1e-6))
        }
    }

    /// Steering input (-1..1 per unit of curvature) that holds a bend at this speed.
    pub fn steer_scalar_at(&self, speed: f32) -> f32 {
        sample(&self.steer_scalar, speed.abs() * 127.0 / self.max_speed.max(1.0))
    }

    /// Full-throttle acceleration at this speed.
    pub fn accel_at(&self, speed: f32) -> f32 {
        sample(&self.accel, speed.abs() * 127.0 / self.max_speed.max(1.0))
    }

    pub fn lerp(a: &AiTune, b: &AiTune, t: f32) -> AiTune {
        let l = |x: f32, y: f32| x + (y - x) * t;
        let lv = |x: &[f32], y: &[f32]| x.iter().zip(y).map(|(p, q)| l(*p, *q)).collect::<Vec<f32>>();
        AiTune {
            max_speed: l(a.max_speed, b.max_speed),
            min_curvature: l(a.min_curvature, b.min_curvature),
            max_curvature: l(a.max_curvature, b.max_curvature),
            max_total_accel: l(a.max_total_accel, b.max_total_accel),
            max_brake_decel: l(a.max_brake_decel, b.max_brake_decel),
            idle_speed: std::array::from_fn(|i| l(a.idle_speed[i], b.idle_speed[i])),
            accel: lv(&a.accel, &b.accel),
            decel: a.decel.iter().zip(&b.decel).map(|(p, q)| lv(p, q)).collect(),
            speed_high_curv: lv(&a.speed_high_curv, &b.speed_high_curv),
            speed_low_curv: lv(&a.speed_low_curv, &b.speed_low_curv),
            steer_scalar: lv(&a.steer_scalar, &b.steer_scalar),
            drag: std::array::from_fn(|i| l(a.drag[i], b.drag[i])),
            accel_rows: a.accel_rows.iter().zip(&b.accel_rows).map(|(p, q)| lv(p, q)).collect(),
        }
    }

    /// [11], set again after the lerp (sub_82549770): 127 / MaxSpeed, the speed tables' index per m/s.
    pub fn speed_index(&self) -> f32 {
        127.0 / self.max_speed
    }

    /// A speed table read as the original's class reads it (sub_8254A280, sub_82549B78 / sub_82549C20):
    /// x = speed * [11], negative to 0; the index floor(min(x, 126)); the fraction x minus that index,
    /// so beyond the table's end it runs on along its last step.
    pub fn table_at(&self, table: &[f32], speed: f32) -> f32 {
        let x = speed * self.speed_index();
        let x = if x > 0.0 { x } else { 0.0 };
        let i = x.min(126.0).floor();
        let k = (i as usize).min(table.len().saturating_sub(2));
        let (a, b) = (table[k], table[k + 1]);
        (b - a).mul_add(x - i, a)
    }
}

/// Stock and fully upgraded.
#[derive(Debug, Clone)]
pub struct AiTunes {
    pub base: AiTune,
    pub mods: AiTune,
}

impl AiTunes {
    pub fn parse(res: &Resource) -> Option<AiTunes> {
        let rd = Rd::new(res);
        let mut found = Vec::new();
        let mut o = 0x50;
        while let Some(c) = rd.ptr(o) {
            o += 4;
            let named = rd.ptr(c + 4).is_some() && rd.ptr(c + 8).is_some();
            if !named && rd.has(c, N * 4) {
                if let Some(t) = AiTune::read(&rd, c) {
                    found.push(t);
                }
            }
            if o > 0x200 {
                break;
            }
        }
        let mut it = found.into_iter();
        let base = it.next()?;
        let mods = it.next().unwrap_or_else(|| base.clone());
        Some(AiTunes { base, mods })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tune(max_speed: f32) -> AiTune {
        AiTune {
            max_speed, min_curvature: 0.0, max_curvature: 0.2, max_total_accel: 10.0, max_brake_decel: 15.0, idle_speed: [0.0; 7],
            accel: Vec::new(), decel: Vec::new(), speed_high_curv: Vec::new(), speed_low_curv: Vec::new(),
            steer_scalar: (0..128).map(|i| i as f32).collect(), drag: [0.0; 3], accel_rows: Vec::new(),
        }
    }

    /// sub_8254A280's read of a speed table: negative speeds at the start, the index held at 126 but
    /// the fraction not - past the end the last step runs on.
    #[test]
    fn table_read_as_the_class() {
        let t = tune(127.0);
        assert_eq!(t.table_at(&t.steer_scalar, -3.0), 0.0);
        assert!((t.table_at(&t.steer_scalar, 10.25) - 10.25).abs() < 1e-5);
        assert!((t.table_at(&t.steer_scalar, 130.0) - 130.0).abs() < 1e-4);
    }
}
