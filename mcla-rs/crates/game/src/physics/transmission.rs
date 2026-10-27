//! Original transmission preparation, pedal modulation and automatic shifts.

pub(super) const RPM_FROM_SPEED: f32 = 9.549_296; // 0x8207D394

#[derive(Clone, Copy, Debug)]
pub(super) struct AutomaticState {
    pub mode: i32,
    /// Port numbering: -1 reverse, 0 neutral, 1.. forward.
    pub gear: i32,
    pub direction: i32,
    pub inhibited: bool,
    pub held: bool,
    pub elapsed: f32,
    pub modulation: [f32; 2],
}

pub(super) struct AutomaticFrame<'a> {
    pub tune: &'a mcla_formats::rage::simtune::TransTune,
    /// The prepared port's table: reverse, then each forward gear (no neutral row).
    pub points: &'a [[f32; 4]],
    pub engine_speed: f32,
    pub pedal: f32,
    pub contacts: &'a [(bool, f32)],
    pub dt: f32,
}

fn approach(current: f32, target: f32, rate: f32, dt: f32) -> f32 {
    if current < target { dt.mul_add(rate, current).min(target) }
    else if current > target { (-dt.mul_add(rate, -current)).max(target) }
    else { current }
}

/// Complete prepared sub_82561AC8, including sub_82560E08 and gear-setting flags.
pub(super) fn update_prepared(mut state: AutomaticState, frame: AutomaticFrame<'_>) -> AutomaticState {
    let tr = &frame.tune.s;
    if state.inhibited { state.elapsed = 0.0; }
    let upper = ((frame.pedal - 0.5) * 2.0).clamp(0.0, 1.0);
    let x = if upper <= 0.25 { 0.0 }
        else if upper >= 0.95 { 1.0 }
        else { (upper - 0.25) * 1.428_571_5 };
    // Original double pow receives this rounded float, then rounds its result.
    let target = if x == 0.0 || x == 1.0 { x } else { (x as f64).powf(5.0) as f32 };
    let factor = target + 1.0;
    let fast = tr.shift_modulator_approach * factor;
    let slow = tr.shift_modulator_decay * factor;
    state.modulation[0] = approach(state.modulation[0], target,
        if target > state.modulation[0] { fast } else { slow }, frame.dt);
    state.modulation[1] = approach(state.modulation[1], target,
        if target < state.modulation[1] { fast } else { slow }, frame.dt);
    if state.gear <= 1 && state.elapsed < tr.gear_change_delay { state.modulation = [1.0; 2]; }
    state.modulation[1] = state.modulation[1].min(state.modulation[0]);

    if state.mode == 1 && state.gear > 0 && !state.held
        && state.elapsed > tr.gear_change_delay && state.direction == 0 {
        let grounded = frame.contacts.iter().any(|c| c.0);
        let grips = frame.contacts.iter().any(|c| c.1 < 0.5);
        let rpm = frame.engine_speed * RPM_FROM_SPEED;
        let g = state.gear as usize;
        let up = frame.points[g];
        let threshold = up[1].mul_add(1.0 - state.modulation[0], up[0] * state.modulation[0]);
        if state.gear < frame.tune.gears as i32 - 2 && rpm > threshold
            && grounded && grips && !state.inhibited {
            state.gear += 1;
            state.direction = 1;
            state.elapsed = 0.0;
        } else if grounded {
            for _ in 0..frame.tune.max_downshifts {
                if state.gear <= 1 { break; }
                let points = frame.points[state.gear as usize];
                let threshold = points[3].mul_add(1.0 - state.modulation[1], points[2] * state.modulation[1]);
                if rpm >= threshold { break; }
                state.gear -= 1;
                state.direction = -1;
                state.elapsed = 0.0;
            }
        }
    }
    // Decisions see the entry timer; even a newly selected gear leaves with dt.
    state.elapsed += frame.dt;
    state
}

/// sub_82560CE0: authored MPH -> wheel RPM, averaging attached wheel radii.
pub(super) fn ratio_for_speed(max_engine_speed: f32, mph: f32, radii: &[f32]) -> f32 {
    let mut wheel_rpm = (26.822_4 * mph) * 0.159_154_94;
    if !radii.is_empty() {
        let mut sum = 0.0;
        for &radius in radii { sum += radius; }
        let mean = sum / radii.len() as f32;
        if mean > 0.0 { wheel_rpm /= mean; }
    }
    (max_engine_speed * RPM_FROM_SPEED) / wheel_rpm
}

/// Original Init first rounds the common gear step, then raises that float
/// step to the biased gear exponent. Both powers receive float inputs promoted
/// to double, and their results round to float before multiplying by low gear.
pub(super) fn ratios(t: &mcla_formats::rage::simtune::TransTune,
                    max_engine_speed: f32, radii: &[f32]) -> Vec<f32> {
    let tr = &t.s;
    let reverse = ratio_for_speed(max_engine_speed, tr.rev_gear_mph, radii);
    let low = ratio_for_speed(max_engine_speed, tr.low_gear_mph, radii);
    let high = ratio_for_speed(max_engine_speed, tr.high_gear_mph, radii);
    let forward = t.gears.max(3) as usize - 2;
    let mut out = vec![reverse];
    if forward == 1 { out.push(high); return out; }
    out.push(low);
    if forward > 2 {
        let span = (forward - 1) as f32;
        let base = high / low;
        let step = (base as f64).powf((1.0 / span) as f64) as f32;
        for i in 1..forward - 1 {
            let i = i as f32;
            let bias = (((i - span) * tr.gear_bias) * i) / span;
            let power = (step as f64).powf((i - bias) as f64) as f32;
            out.push(power * low);
        }
    }
    out.push(high);
    out
}

#[cfg(test)]
mod automatic_tests {
    use super::*;

    #[test]
    fn captured_vantage_upshift_and_strict_threshold_preserve_original_timer_phase() {
        let mut tune = mcla_formats::rage::simtune::TransTune::default();
        tune.gears = 8; tune.max_downshifts = 3;
        tune.s.gear_change_delay = 0.8;
        tune.s.shift_modulator_approach = 0.4; tune.s.shift_modulator_decay = 0.2;
        let points = [
            [0.0; 4], [7175.31348, 4305.18848, 0.0, 0.0],
            [7123.02783, 4273.81689, 4121.44336, 2289.69092],
            [7063.44727, 4238.06836, 4381.97607, 2434.43115],
            [6991.70703, 4195.02441, 4653.91992, 2585.51123],
            [6893.8252, 4136.29541, 4933.80859, 2741.00488],
            [7800.0, 7800.0, 5210.22217, 2894.56812],
        ];
        let state = AutomaticState { mode: 1, gear: 3, direction: 0, inhibited: false,
            held: false, elapsed: 0.816666663, modulation: [1.0; 2] };
        let contacts = [(true, 0.1); 2];
        let step = |state, speed| update_prepared(state, AutomaticFrame { tune: &tune,
            points: &points, engine_speed: speed, pedal: 1.0, contacts: &contacts, dt: 1.0/60.0 });
        // Original captures 2 and 1: first changes 3 -> 4, second is exactly on
        // second gear's threshold and must remain in second.
        let shifted = step(state, 739.683228);
        assert_eq!((shifted.gear, shifted.direction), (4, 1));
        assert_eq!(shifted.elapsed.to_bits(), (1.0f32/60.0).to_bits());
        assert_eq!(shifted.modulation, [1.0; 2]);
        let at_threshold = step(AutomaticState { gear: 2, ..state }, 745.921753);
        assert_eq!((at_threshold.gear, at_threshold.direction), (2, 0));
        assert_eq!(at_threshold.elapsed.to_bits(), 0.833333313f32.to_bits());
        // Entry delay is inclusive; it is advanced only after the decision.
        let delayed = step(AutomaticState { elapsed: 0.8, ..state }, 739.683228);
        assert_eq!((delayed.gear, delayed.direction), (3, 0));
        assert_eq!(delayed.elapsed.to_bits(), 0.816666663f32.to_bits());
    }

    #[test]
    // (2,032 of every eighth of the 16,384 cases of tools/automatic_transmission_reference.py, the original's Update run offline:
    // testdata/automatic_update_ref.csv)
    fn automatic_update_matches_complete_original_function() {
        let text = include_str!("testdata/automatic_update_ref.csv");
        let mut mismatches = Vec::new();
        let mut count = 0;
        for (case, line) in text.lines().skip(1).enumerate() {
            let a: Vec<f32> = line.split(',').map(|v| v.parse().unwrap()).collect();
            assert_eq!(a.len(), 66);
            let port_gear = |g: i32| if g == 0 { -1 } else if g == 1 { 0 } else { g - 1 };
            let input = AutomaticState { mode: a[0] as i32, gear: port_gear(a[1] as i32),
                direction: a[2] as i32, inhibited: a[3] != 0.0, held: a[4] != 0.0,
                elapsed: a[5], modulation: [a[8], a[9]] };
            let mut tune = mcla_formats::rage::simtune::TransTune::default();
            tune.gears = a[10] as u32; tune.max_downshifts = a[11] as u32;
            tune.s.gear_change_delay = a[12]; tune.s.shift_modulator_approach = a[13]; tune.s.shift_modulator_decay = a[14];
            let points: Vec<[f32; 4]> = (0..tune.gears as usize - 1).map(|i| {
                let original = if i == 0 { 0 } else { i + 1 };
                a[25+original*4..29+original*4].try_into().unwrap()
            }).collect();
            let contacts: Vec<_> = (0..a[16] as usize).map(|i| (a[21+i] != 0.0, a[17+i])).collect();
            let out = update_prepared(input, AutomaticFrame { tune: &tune, points: &points,
                engine_speed: a[7], pedal: a[6], contacts: &contacts, dt: a[15] });
            let actual = [out.elapsed, out.modulation[0], out.modulation[1]].map(f32::to_bits);
            let expected = [a[63], a[64], a[65]].map(f32::to_bits);
            if out.gear != port_gear(a[61] as i32) || out.direction != a[62] as i32 || actual != expected {
                mismatches.push((case, out, [a[61], a[62], a[63], a[64], a[65]]));
            }
            count += 1;
        }
        assert_eq!(count, 2032);
        assert!(mismatches.is_empty(), "{} mismatches; first {:?}", mismatches.len(), &mismatches[..mismatches.len().min(3)]);
    }
}
