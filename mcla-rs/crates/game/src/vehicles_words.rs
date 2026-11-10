//! Small routines of the vehicle code (lamps, paint, resets) in the form the original has them: the same operations in the
//! same order, the scalar unit's fused ones where the original fuses, compared with the generated code run offline
//! (`vehicles_small_tests`, tools/vehicles_small_reference.py). Where the play code has the same arithmetic it calls these.

use crate::physics::exact::fma;

/// F-44484, the paint rows: pow(x, 2.2) of the three colour channels (the exponent the image's double: the single 2.2
/// widened, 2.200000047..., the original's own pow, `caraudio::pow_guest`), the fourth word passed through.
pub fn paint_pow(c: [f32; 4]) -> [f32; 4] {
    let p = |x: f32| crate::caraudio::pow_guest(x as f64, 2.2_f32 as f64) as f32;
    [p(c[0]), p(c[1]), p(c[2]), c[3]]
}

/// F-75961, an ambient car's lights-on getter from what it reads: `fade` the car's +484, `has_slot` the object at +452 that
/// is asked (virtual slot 72) and `slot_on` its answer (a yes puts the lights out), the light manager's `sunrise` / `sunset`
/// (+14564 / +14568) and the `hour`, the car's own `offset` (+3568) and the weather terms' sum (the viewport table's +8 and
/// +12 of the thread's row, `weather`). Off under 0.1 fade (a NaN passes); on past (sunset - 0.333) - offset, before
/// offset + (sunrise + 0.5), or while the offset is under the weather sum.
pub fn lights_on(fade: f32, has_slot: bool, slot_on: bool, sunrise: f32, sunset: f32, hour: f32, offset: f32, weather: f32) -> bool {
    if fade < 0.1 || !has_slot || slot_on {
        return false;
    }
    let before_sunset = sunset - 0.333;
    let after_sunrise = sunrise + 0.5;
    hour > before_sunset - offset || hour < offset + after_sunrise || offset < weather
}

/// F-40379, the nitro counter's reset: (most +14, count +18) = the smaller (signed 16 bits) of the cap (+16) and the tune's
/// shot count, then burning (+32) = 0 and the level / view floats (+24, +28) = 0.
pub fn nitro_count(cap: i16, tune: i16) -> i16 {
    if cap < tune {
        cap
    } else {
        tune
    }
}

/// F-42929, the window controller's reset: the words it writes (+8 state 0 = "take the target", +12 target 1 = closed,
/// +76 hold counter 3, +64 time 0.0) and whether it stops the sound object at +20 (when there is one: F-31667 with 0).
#[derive(Debug, PartialEq)]
pub struct WindowReset {
    pub state: u32,
    pub target: u32,
    pub hold: u32,
    pub time: f32,
    pub stop_sound: bool,
}

pub fn window_reset(has_sound: bool) -> WindowReset {
    WindowReset { state: 0, target: 1, hold: 3, time: 0.0, stop_sound: has_sound }
}

/// F-96984's table of lamp states by attachment, from the six eased states (head +168, the second head state +188, brake
/// +208, siren +308, flash +248, reversing +228): 0 `fma(head, 0.25, brake x 0.75)`, 1 head x 0.25, 2 brake, 3 the sum of
/// head and flash held between 0 and 1 by two `fsel`s (a NaN gives 0), 4 flash, 5 the second head state, 6 head, 7 .. 9
/// siren, 11 reversing, the rest 0.
pub fn setup_table(head: f32, head2: f32, brake: f32, siren: f32, flash: f32, reversing: f32) -> [f32; 17] {
    let sum = flash + head;
    let low = if sum >= 0.0 { sum } else { 0.0 };
    let sat = if low - 1.0 >= 0.0 { 1.0 } else { low };
    [fma(head, 0.25, brake * 0.75), head * 0.25, brake, sat, flash, head2, head, siren, siren, siren, 0.0, reversing, 0.0, 0.0, 0.0, 0.0, 0.0]
}

/// F-96984's car +2676: pow(level 0 x 1.5, 2.2) (the original's pow, the exponent the single 2.2 widened).
pub fn setup_glow(level0: f32) -> f32 {
    crate::caraudio::pow_guest((level0 * 1.5) as f64, 2.2_f32 as f64) as f32
}

/// F-96984's car +2680: ((pulse x the neon's intensity) x car +268) x 0.3 (the interior's +2984 is ((flash x the record's
/// +308) x car +268), no 0.3).
pub fn neon_scale(pulse: f32, intensity: f32, one: f32) -> f32 {
    ((pulse * intensity) * one) * f32::from_bits(0x3e99_999a)
}

/// F-81135's traffic flags: the byte at car +557 + thread index, bits 0x4 head, 0x40 brake, 0x20 and 0x10 the signal sides,
/// 0x8 reversing.
pub fn traffic_flags(b: u8) -> [bool; 5] {
    [b & 0x4 != 0, b & 0x40 != 0, b & 0x20 != 0, b & 0x10 != 0, b & 0x8 != 0]
}

/// F-81135's table by attachment, from the five eased states (head, brake, side a, side b, reversing): 0 `fma(head, 0.25,
/// brake x 0.75)`, 1 head x 0.25, 2 brake, 3 and 5 head, 7 .. 9 one, 11 reversing, 13 and 15 side a, 14 and 16 side b, the rest 0.
pub fn traffic_table(s: [f32; 5]) -> [f32; 17] {
    let [h, b, sa, sb, r] = s;
    [fma(h, 0.25, b * 0.75), h * 0.25, b, h, 0.0, h, 0.0, 1.0, 1.0, 1.0, 0.0, r, 0.0, sa, sb, sa, sb]
}

/// F-81135's glow fade by the camera depth `d`: (1 - sat((d - 50) x (1 / 325)))^2 with the two `fsel`s (a NaN gives 1), and 1
/// exactly at the largest float (the "no depth" word).
pub fn traffic_fade(d: f32) -> f32 {
    let x = if d == f32::MAX {
        1.0
    } else {
        let t = (d - 50.0) * f32::from_bits(0x3b49_a634);
        let low = if t >= 0.0 { t } else { 0.0 };
        let sat = if low - 1.0 >= 0.0 { 1.0 } else { low };
        1.0 - sat
    };
    x * x
}

/// F-81135's glow level of an entry: ((state x fade) x the thread slot's scale) x the type's factor (+84).
pub fn traffic_level(state: f32, fade: f32, scale: f32, factor: f32) -> f32 {
    ((state * fade) * scale) * factor
}

/// F-12794, the engine's speed in revs per minute: its +8 (rad/s) x 9.5493 (the image's single, 0x4118c9eb).
pub fn engine_rpm(engine_w: f32) -> f32 {
    engine_w * f32::from_bits(0x4118_c9eb)
}

/// F-10129's shift light: lit unless the engine's speed (`engine_rpm`) is under the gear's shift point (a NaN is lit).
pub fn shift_light(rpm: f32, point: f32) -> bool {
    !(rpm < point)
}

/// What F-10129 reads, in the order of its reads: the car's lamp switch (+597), the driver input's brake pedal when the object
/// exists (+16), the stealth check's answer (false when the sim object has no +128), whether the gear index is 0 (reverse is
/// the first gear), the vehicle's +224 speed, the light manager's garage byte (14546), the hour and its sunrise / sunset, the
/// viewport row's two weather weights (+12 `storm`, +8 `fog`), the garage object's cabin-view byte (39229), the
/// "always rain" debug byte, the engine speed in rpm (F-12794's answer), the gear's shift point, the record's two neon rates (bytes 396 and 397,
/// signed) and the unpaused clock.
pub struct LampIn {
    pub switch: bool,
    pub pedal: Option<f32>,
    pub stealth: bool,
    pub reverse_gear: bool,
    pub speed: f32,
    pub garage: bool,
    pub hour: f32,
    pub sunrise: f32,
    pub sunset: f32,
    pub storm: f32,
    pub fog: f32,
    pub cabin: bool,
    pub always_rain: bool,
    pub rpm: f32,
    pub shift_point: f32,
    pub ext_rate: i8,
    pub int_rate: i8,
    pub clock: f32,
}

/// What F-10129 sets before it eases: the head target (+164, copied to +184), brake (+204), reversing (+224), shift
/// (+388), the speed target (+368, only with the driver input), the lamp set's byte (2161 or 2162 set), the neon flashes
/// (+404 exterior, +408 interior), the debug flash.
pub struct LampOut {
    pub head: bool,
    pub brake: bool,
    pub reverse: bool,
    pub shift: bool,
    pub speed_target: Option<f32>,
    pub set_byte: u16,
    pub neon: [f32; 2],
    pub rain_flash: bool,
}

pub fn lamp_targets(i: &LampIn) -> LampOut {
    let stealth = i.stealth;
    let brake = i.switch && i.pedal.is_some_and(|p| p > f32::from_bits(0x3a83_126f)) && !stealth;
    let reverse = i.switch && i.reverse_gear && !stealth;
    let speed_target = i.pedal.map(|p| {
        let t = fma(i.speed, f32::from_bits(0x400f_2b02), -3.0) * f32::from_bits(0x3c88_8889);
        let low = if t >= 0.0 { t } else { 0.0 };
        p * if low - 1.0 >= 0.0 { 1.0 } else { low }
    });
    let storm = i.storm > f32::from_bits(0x3f59_999a);
    let fog = i.fog > f32::from_bits(0x3f59_999a);
    let (mut head, set_byte) = if i.garage {
        if i.cabin {
            (true, 2161)
        } else {
            (false, 2162)
        }
    } else {
        let sun = i.hour > i.sunset || i.hour < i.sunrise;
        if !sun && !fog && !storm {
            (false, 2162)
        } else {
            (true, 2161)
        }
    };
    if !i.switch || stealth {
        head = false;
    }
    LampOut {
        head,
        brake,
        reverse,
        shift: shift_light(i.rpm, i.shift_point),
        speed_target,
        set_byte,
        neon: [crate::drivers::neon_flash(i.ext_rate as f32, i.clock), crate::drivers::neon_flash(i.int_rate as f32, i.clock)],
        rain_flash: i.always_rain,
    }
}
