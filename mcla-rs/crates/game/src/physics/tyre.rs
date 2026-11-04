//! Contact-frame scalar tyre update from 0x825668D8..0x82566C20.
//! Contact/suspension and chassis force application live outside this kernel.

use mcla_formats::rage::simtune::WheelTune;
use super::vehsim::{displace, friction, slip};

#[derive(Clone, Copy, Debug, Default)]
pub struct Contact {
    pub radius: f32,
    /// Original sign: negative rolling forward.
    pub spin: f32,
    pub velocity_lat: f32,
    pub velocity_long: f32,
    pub load: f32,
    pub surface: f32,
    pub spring_lat: f32,
    pub spring_long: f32,
    pub damper_lat: f32,
    pub damper_long: f32,
    pub dt: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct Forces {
    pub lateral: f32,
    pub longitudinal: f32,
    pub slip_lat: f32,
    pub slip_long: f32,
    pub slide: f32,
    /// the two friction evaluations' own slide values (0 where the tyre held), and whether
    /// each displacement took its whole step
    pub slide_lat: f32,
    pub slide_long: f32,
    pub grip_lat: bool,
    pub grip_long: bool,
}

fn signed_unit(velocity: f32, spring: f32, load: f32) -> f32 {
    // fsel gives +0 for either sign of zero velocity; f32::signum gives +/-1.
    let sign = if velocity > 0.0 { 1.0 } else if velocity < 0.0 { -1.0 } else { 0.0 };
    (sign / spring) * load
}

fn within(disp: f32, limit: f32, step: f32) -> bool {
    if step > 0.0 { disp < limit && step <= limit - disp }
    else { disp > limit && step >= limit - disp }
}

/// Update displacement springs and apply the original 2D friction circle.
pub fn update(tune: &WheelTune, (wheel_static, sliding): (f32, f32), c: Contact,
              displacement_lat: &mut f32, displacement_long: &mut f32) -> Forces {
    let rolling = c.spin * c.radius;
    let patch = c.velocity_long + rolling;
    let slip_lat = slip(c.velocity_lat, c.velocity_long);
    let slip_long = slip(patch, c.velocity_long);
    let static_mu = tune.static_fric * c.surface;
    let opt = tune.optimum_slip;
    let unit_lat = signed_unit(c.velocity_lat, c.spring_lat, c.load);
    let unit_long = signed_unit(patch, c.spring_long, c.load);
    let step_lat = c.velocity_lat * c.dt;
    let step_long = patch * c.dt;
    let (mu_long, slide_long) = if slip_long.abs() > opt &&
        within(*displacement_long, static_mu * unit_long, step_long) {
        (static_mu, 0.0)
    } else { friction(tune, wheel_static, sliding, c.surface, slip_long) };
    let (mu_lat, slide_lat) = if slip_lat.abs() > opt &&
        within(*displacement_lat, static_mu * unit_lat, step_lat) {
        (static_mu, 0.0)
    } else { friction(tune, wheel_static, sliding, c.surface, slip_lat) };
    let mut limit_lat = mu_lat * unit_lat;
    let mut limit_long = mu_long * unit_long;
    let mut mu = static_mu;
    if slip_lat.abs().max(slip_long.abs()) >= opt {
        mu = if slip_long.abs() >= slip_lat.abs() { mu_long } else { mu_lat };
        if mu_lat > mu { limit_lat = mu * unit_lat; }
        if mu_long > mu { limit_long = mu * unit_long; }
    }
    let relax = (rolling.abs() * c.dt) * 0.1;
    let (applied_long, grip_long) = displace(displacement_long, limit_long, step_long, relax);
    let (applied_lat, grip_lat) = displace(displacement_lat, limit_lat, step_lat, relax);
    let inverse_step = 1.0 / c.dt;
    let mut lateral = -(applied_lat * inverse_step).mul_add(c.damper_lat, c.spring_lat * *displacement_lat);
    let mut longitudinal = -(applied_long * inverse_step).mul_add(c.damper_long, c.spring_long * *displacement_long);
    let cap = c.load * mu;
    let cap2 = cap * cap;
    let magnitude2 = lateral.mul_add(lateral, longitudinal * longitudinal);
    if magnitude2 > cap2 {
        if (!grip_long && slip_long.abs() > opt) || (!grip_lat && slip_lat.abs() > opt) {
            let velocity2 = patch.mul_add(patch, c.velocity_lat * c.velocity_lat);
            let scale = -(cap2 / velocity2).sqrt();
            lateral = c.velocity_lat * scale;
            longitudinal = patch * scale;
        } else {
            let scale = (cap2 / magnitude2).sqrt();
            lateral *= scale;
            longitudinal *= scale;
        }
    }
    Forces { lateral, longitudinal, slip_lat, slip_long, slide: slide_lat.max(slide_long), slide_lat, slide_long, grip_lat, grip_long }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn friction_circle_and_elastic_state_match_original_captures() {
        // The zero-lateral-velocity capture retains elastic spring force. Zero
        // velocity alone must not reset displacement or force to zero.
        let fixtures = [
            "200000,1000,200000,0,0.289000005,1.85000002,1.60000002,0.0700000003,1,1000,5,30,-114.186852,0.100000001,0.00999999978,0.0333333351,0.00800000038,-0.00800000038,-1371.98865,823.193237,0.166666672,-0.100000001",
            "200000,100,200000,100,0.289000005,1.85000002,0,0.540000021,0.800000012,3000,0,1,-3.11418676,-0.00999999978,0.00999999978,0.00416666688,-0.00999999978,0.00962499995,2000,-1925,0,0.100000024",
        ];
        for line in fixtures { compare_capture(line); }
    }

    fn compare_capture(line: &str) {
        let p: Vec<f32> = line.split(',').map(|s| s.parse().unwrap()).collect();
        assert_eq!(p.len(), 22);
        let tune = WheelTune { radius: p[4], static_fric: p[5], sliding_fric: p[6],
                              optimum_slip: p[7], ..Default::default() };
        let c = Contact { spring_lat: p[0], damper_lat: p[1], spring_long: p[2], damper_long: p[3],
            radius: p[4], surface: p[8], load: p[9], velocity_lat: p[10], velocity_long: p[11],
            spin: p[12], dt: p[15] };
        let (mut dl, mut df) = (p[13], p[14]);
        let result = update(&tune, (p[5], p[6]), c, &mut dl, &mut df);
        for (a, e) in [dl, df, result.lateral, result.longitudinal, result.slip_lat, result.slip_long]
            .into_iter().zip(p[16..].iter().copied()) {
            assert_eq!(a.to_bits(), e.to_bits(), "{line}: {result:?}, displacement {dl}, {df}");
        }
    }

    #[test]
    #[ignore = "requires MCLA_TYRE_REFERENCE from tools/tyre_reference.py"]
    fn contact_tyre_update_matches_original_generated_code() {
        let path = std::env::var("MCLA_TYRE_REFERENCE").expect("MCLA_TYRE_REFERENCE");
        let csv = std::fs::read_to_string(path).unwrap();
        let mut count = 0;
        for line in csv.lines().skip(1) {
            compare_capture(line);
            count += 1;
        }
        assert_eq!(count, 8000);
        println!("{count} original 2D tyre cases match exactly");
    }
}
