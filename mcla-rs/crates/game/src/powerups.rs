//! Power-ups in a race (step 19e) - the original's pick-up manager (mcPickUpManager /
//! mcPowerUpSource, 0x8267F000..0x82687000), the car's holder (mcCarPickups, 0x8232A1B0..
//! 0x8232D000) and the power-up types (mcPowerUpType*, 0x8237C370..0x82380800), read from the
//! executable. See `D:\MCLA\SAVE_WORK.md`, "Step 19e".
//!
//! Where they appear: the race cores (OrderedCheckpointCore `game/0x3ae26fc8/0xf637b18a` 0x38d,
//! the unordered core `0xd82fca9b`) make the sources when `EventOptions_IsPowerUps` (the
//! options record's byte 21) is set - offline that is an ARCADE race with POWER UPS on or
//! Custom. The race script's own list (`PickUpManager_CreatePowerUpSources(n)` +
//! `PickUpManager_AddPowerUpSource(list, pos, width, second)` in a loop) is used when it has
//! one; otherwise the sources are laid out from the checkpoints (sub_826830A0, 1 for the
//! ordered core, 0 for the unordered one). `PickUpManager_OnRaceReset` (sub_82680328) then
//! gives every source its type.

use glam::{Quat, Vec3};

pub const NITRO: u8 = 0;
pub const ICE: u8 = 1;
pub const REVERSE: u8 = 2;
pub const STEALTH: u8 = 3;
pub const DISRUPTER: u8 = 4;
pub const SHIELD: u8 = 5;
pub const STOP: u8 = 6;
pub const PULSE: u8 = 7;
pub const MIRROR: u8 = 8;
pub const AGRO: u8 = 9;
pub const MEGA_STEALTH: u8 = 14;
pub const MEGA_SHIELD: u8 = 16;
pub const MEGA_MIRROR: u8 = 19;
pub const MEGA_AGRO: u8 = 20;
pub const MYSTERY: u8 = 22;
/// "InvalidPowerup": an empty slot, a source with no type
pub const NONE: u8 = 25;

/// Step 19g: how long a laser fan fades after its end (the shoot-effect manager's +1400)
pub const LASER_OUT: f32 = 0.25;

/// The type names (table 0x82828260): the sections of the manager's tune file and the
/// `powerup_<name>.mcpowerup` look files.
pub const NAMES: [&str; 24] = [
    "Nitro", "Ice", "ReverseSteer", "StealthMode", "Disrupter", "Shield", "Stop", "Pulse", "Mirror", "Agro", "HotPotato",
    "MegaNitro", "MegaIce", "MegaReverseSteer", "MegaStealthMode", "MegaDisrupter", "MegaShield", "MegaStop", "MegaPulse",
    "MegaMirror", "MegaAgro", "MegaHotPotato", "Mystery", "HitByAgro",
];

/// One power-up type: the constructors' values (0x8237C460 and the per-type ones, called by
/// the manager's constructor sub_8273FC88) with the tune file's (`tune/powerup/0xe49d3ba2`,
/// mcPowerUpMgr, parsed by sub_8237C950 / sub_8237EAE0) laid over them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TypeTune {
    /// +16 ActiveTime: how long the effect lasts on a car
    pub active_time: f32,
    /// +20 Shootable: fired at another car (else used on the own car)
    pub shootable: bool,
    /// +24 ShootTime: how long a shot looks for a target
    pub shoot_time: f32,
    /// +28 ShootRange
    pub shoot_range: f32,
    /// +32 the half size of the box ahead a target must be in (10; the megas -1: any
    /// direction, by distance)
    pub width: f32,
    /// +8 the type two of these make (25 none)
    pub mega: u8,
    /// Pulse: +112 Force, +128 Torque, +144 ForceDir
    pub force: Vec3,
    pub torque: Vec3,
    pub force_dir: Vec3,
    /// step 19g: +44 FadeFxStart, +48 FadeFxFrequency, +52 FadeFxAmplitude, +56
    /// FadeFxAcceleration (the constructor's 0.7 / 20 / 4 / 2; every type of the file the same)
    pub fade: [f32; 4],
    /// +36 DrawIcon
    pub draw_icon: bool,
}

impl TypeTune {
    /// The constructors' values (sub_8237C460 base: ActiveTime 10, not shootable, ShootTime 5,
    /// ShootRange 20, +32 10, no mega; then sub_8237CCB8 Ice .. sub_8237E790 MegaHotPotato).
    pub fn constructed(t: u8) -> TypeTune {
        let mut v = TypeTune { active_time: 10.0, shootable: false, shoot_time: 5.0, shoot_range: 20.0, width: 10.0, mega: NONE, force: Vec3::ZERO, torque: Vec3::ZERO, force_dir: Vec3::ZERO, fade: [0.7, 20.0, 4.0, 2.0], draw_icon: true };
        let (mega, shoot, megawidth) = match t {
            1 | 12 => (12, true, t == 12),
            2 | 13 => (13, true, t == 13),
            3 | 14 => (14, false, false),
            4 | 15 => (15, true, t == 15),
            5 | 16 => (16, false, false),
            6 | 17 => (17, true, t == 17),
            7 | 18 => (18, true, t == 18),
            8 | 19 => (19, false, false),
            9 | 20 => (20, false, false),
            _ => (NONE, false, false),
        };
        v.mega = mega;
        v.shootable = shoot;
        if megawidth {
            v.width = -1.0;
        }
        v
    }
}

/// The look of a source (`tune/powerup/powerup_<name>.mcpowerup`, mcPowerup, parser
/// sub_82681348): the colours and heights the original's draw uses.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Look {
    pub glow_color: [f32; 4],
    pub sphere_color: [f32; 4],
    pub sphere_scale: f32,
    pub powerup_height: f32,
    /// step 19f, what the draw (sub_82681760 / sub_82682318) reads - the object's +28
    /// m_groundGlowScale, +32 m_powerupWidth, +48 m_iconHeight, +52 m_powerupHeight, +112
    /// m_powerupColor, +128 / +144 m_groundGlowColor1 / 2 (field table sub_82681358; defaults
    /// of the constructor sub_826811D0 where a file leaves one out)
    pub ground_glow_scale: f32,
    pub powerup_width: f32,
    pub icon_height: f32,
    pub powerup_color: [f32; 4],
    pub ground_glow_color1: [f32; 4],
    pub ground_glow_color2: [f32; 4],
    /// step 19g: +192 m_beamColor (the shot's sparks' tint, the beam's colour)
    pub beam_color: [f32; 4],
}

/// Values of a text tune file ("type: a", `Name { key v v .. }` blocks) by "section.key"
/// (the top block's keys by "key").
pub fn parse_tune(text: &str) -> std::collections::HashMap<String, Vec<f32>> {
    let mut out = std::collections::HashMap::new();
    let mut stack: Vec<String> = Vec::new();
    for line in text.lines() {
        let l = line.trim();
        if l.is_empty() || l.starts_with("type:") {
            continue;
        }
        if l == "}" {
            stack.pop();
            continue;
        }
        let mut w = l.split_whitespace();
        let Some(key) = w.next() else { continue };
        let rest: Vec<&str> = w.collect();
        if rest.last() == Some(&"{") {
            stack.push(key.to_string());
            continue;
        }
        // (the outermost block is the class itself)
        let path = if stack.len() > 1 { format!("{}.{}", stack[1..].join("."), key) } else { key.to_string() };
        out.insert(path, rest.iter().filter_map(|x| x.parse::<f32>().ok()).collect());
    }
    out
}

/// The 24 types with the tune file laid over the constructors.
pub fn load_types(data_dir: &str) -> [TypeTune; 24] {
    let mut types: [TypeTune; 24] = std::array::from_fn(|t| TypeTune::constructed(t as u8));
    let path = std::path::Path::new(data_dir).join("tune/powerup/0xe49d3ba2");
    let Ok(text) = std::fs::read_to_string(&path) else {
        log::warn!("powerups: no manager tune {}", path.display());
        return types;
    };
    let v = parse_tune(&text);
    for (t, name) in NAMES.iter().enumerate() {
        let f = |k: &str| v.get(&format!("{name}.{k}")).and_then(|x| x.first().copied());
        let v3 = |k: &str| v.get(&format!("{name}.{k}")).filter(|x| x.len() >= 3).map(|x| Vec3::new(x[0], x[1], x[2]));
        let ty = &mut types[t];
        if let Some(x) = f("ActiveTime") {
            ty.active_time = x;
        }
        if let Some(x) = f("Shootable") {
            ty.shootable = x != 0.0;
        }
        if let Some(x) = f("ShootTime") {
            ty.shoot_time = x;
        }
        if let Some(x) = f("ShootRange") {
            ty.shoot_range = x;
        }
        if let Some(x) = v3("Force") {
            ty.force = x;
        }
        if let Some(x) = v3("Torque") {
            ty.torque = x;
        }
        if let Some(x) = v3("ForceDir") {
            ty.force_dir = x;
        }
        for (i, k) in ["FadeFxStart", "FadeFxFrequency", "FadeFxAmplitude", "FadeFxAcceleration"].iter().enumerate() {
            if let Some(x) = f(k) {
                ty.fade[i] = x;
            }
        }
        if let Some(x) = f("DrawIcon") {
            ty.draw_icon = x != 0.0;
        }
    }
    types
}

/// Step 19g: the effect's fade (sub_8237CAC8, type +44 .. +56): 1 until the last
/// (1 - FadeFxStart) of the effect's time, then with r = (time left / ActiveTime) / (1 -
/// FadeFxStart) it flickers out: clamp((1 + Amplitude cos(Frequency (1 - r)^Acceleration)) r, 0, 1)
/// (a negative Acceleration takes r^-Acceleration instead). No effect on the car: 1.
pub fn fade_fx(t: &TypeTune, left: f32) -> f32 {
    let f = left / t.active_time;
    let span = 1.0 - t.fade[0];
    let r = if f <= 0.0 {
        0.0
    } else if f >= span {
        return 1.0;
    } else {
        f / span
    };
    if r == 1.0 {
        return 1.0;
    }
    let p = if t.fade[3] >= 0.0 { (1.0 - r).powf(t.fade[3]) } else { r.powf(-t.fade[3]) };
    ((1.0 + t.fade[2] * (t.fade[1] * p).cos()) * r).clamp(0.0, 1.0)
}

/// Step 19g: how an effect changes the car's OWN draw - (DL6.w, stealth):
/// - Mirror / MegaMirror (the draw +40, sub_823802D8): the car's render object +4108 =
///   clamp(fade, 0, 1); sub_823580D0 hands it to the paint as DL6.w (handle +88, sub_823798D8) for
///   the player's car only (any other car: 0) and PS_CarPaint adds it to its mirror weight
///   (instruction 111: lerp(paint x light, reflection, weight + DL6.w));
/// - Stealth / MegaStealth (sub_822A91E0 -> sub_8232AF88 / sub_8232AFB0) while the fade is above
///   0 (sub_8235FB78 0x8235FDFC..0x8235FE38): the car is not drawn in the opaque pass and is
///   drawn in the alpha pass with powerupicecube's "stealthtechnique" forced (the type's +48,
///   sub_8237D568) - pu_fx.rs.
pub fn car_look(effect: Option<(u8, f32)>, types: &[TypeTune; 24], player: bool) -> (f32, bool) {
    let Some((kind, left)) = effect else { return (0.0, false) };
    let Some(t) = types.get(kind as usize) else { return (0.0, false) };
    match kind {
        MIRROR | MEGA_MIRROR => (if player { fade_fx(t, left).clamp(0.0, 1.0) } else { 0.0 }, false),
        STEALTH | MEGA_STEALTH => (0.0, fade_fx(t, left) > 0.0),
        _ => (0.0, false),
    }
}

/// Step 19g: the lightning's settings of Reverse, Disrupter, Stop and their megas (the types'
/// ma_ / mi_ blocks - Reverse at +112 / +224, Disrupter and Stop at +3472 / +3584 - and the
/// manager's bolts).
pub fn load_bolt_tunes(data_dir: &str) -> Vec<(u8, crate::pu_bolt::BoltTune, [crate::pu_bolt::ArcTune; 2])> {
    let path = std::path::Path::new(data_dir).join("tune/powerup/0xe49d3ba2");
    let Ok(text) = std::fs::read_to_string(&path) else { return Vec::new() };
    let v = parse_tune(&text);
    [2u8, 4, 6, 13, 15, 17]
        .iter()
        .map(|&k| {
            let (b, a) = crate::pu_bolt::load_tune(&v, NAMES[k as usize]);
            (k, b, a)
        })
        .collect()
}

/// The looks of the 24 types (`powerup_<name>.mcpowerup`, stored under its hash).
pub fn load_looks(data_dir: &str) -> [Look; 24] {
    std::array::from_fn(|t| {
        let file = format!("0x{:08x}", mcla_formats::hash::joaat(&format!("powerup_{}.mcpowerup", NAMES[t])));
        let Ok(text) = std::fs::read_to_string(std::path::Path::new(data_dir).join("tune/powerup").join(file)) else { return Look::default() };
        let v = parse_tune(&text);
        let c = |k: &str| v.get(k).filter(|x| x.len() >= 4).map(|x| [x[0], x[1], x[2], x[3]]).unwrap_or([1.0; 4]);
        let f = |k: &str, d: f32| v.get(k).and_then(|x| x.first().copied()).unwrap_or(d);
        let cd = |k: &str, d: [f32; 4]| v.get(k).filter(|x| x.len() >= 4).map(|x| [x[0], x[1], x[2], x[3]]).unwrap_or(d);
        Look {
            glow_color: c("m_glowColor"),
            sphere_color: c("m_sphereColor"),
            sphere_scale: f("m_sphereScale", 1.0),
            powerup_height: f("m_powerupHeight", 1.0),
            ground_glow_scale: f("m_groundGlowScale", 1.0),
            powerup_width: f("m_powerupWidth", 1.0),
            icon_height: f("m_iconHeight", 2.0),
            powerup_color: cd("m_powerupColor", [1.0; 4]),
            ground_glow_color1: cd("m_groundGlowColor1", [0.0, 1.0, 1.0, 0.24]),
            ground_glow_color2: cd("m_groundGlowColor2", [0.0, 1.0, 1.0, 0.0]),
            // (the constructor F-88402's default; 5 look files have no m_beamColor)
            beam_color: cd("m_beamColor", [0.372, 0.702, 0.831, 1.0]),
        }
    })
}

// ---------------------------------------------------------------------------------
// Sources

/// A place where a source is to stand: probed straight down from `at` (`up` above, `down`
/// below); `height` how far above it a car may be and still take it (+112), `last` the
/// checkpoint layout's copy for the last checkpoint (+21).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spot {
    pub at: Vec3,
    pub up: f32,
    pub down: f32,
    pub height: f32,
    pub last: bool,
}

/// `PickUpManager_AddPowerUpSource(list, pos, width, second)` (sub_82680130): a zero `second`
/// makes a ring (sub_82682970), otherwise a row across the way from `pos` to `second`
/// (sub_82682BF8). Each spot is probed from 2 m above to 20 m below; the height is 5.
pub fn hand_spots(pos: Vec3, width: f32, second: Vec3) -> Vec<Spot> {
    let spot = |p: Vec3| Spot { at: p, up: 2.0, down: 20.0, height: 5.0, last: false };
    if second == Vec3::ZERO {
        // a ring of radius `width`: 5 sources from 15 m, 4 from 10 m, else 3
        let n = if width >= 15.0 { 5 } else if width >= 10.0 { 4 } else { 3 };
        return (0..n)
            .map(|i| {
                let a = (i as f32 / n as f32 * 360.0) * 0.017453292;
                spot(pos + Vec3::new(a.cos() * width, 0.0, -a.sin() * width))
            })
            .collect();
    }
    // the way from pos to second, normalised, turned a quarter round the up axis
    let d = second - pos;
    let d = if d.length_squared() > 0.0 { d.normalize() } else { d };
    let d = Vec3::new(d.z, d.y, -d.x);
    // 5 from 13 m, 4 from 10, 3 from 6.5, 2 from 3, else 1 in the middle; spread over
    // -width .. +width
    let n = if width >= 13.0 { 5 } else if width >= 10.0 { 4 } else if width >= 6.5 { 3 } else if width >= 3.0 { 2 } else { 1 };
    let (start, step) = if n == 1 { (0.0, 0.0) } else { (-width, width * 2.0 / (n - 1) as f32) };
    (0..n).map(|i| spot(pos + d * (start + step * i as f32))).collect()
}

/// The lateral places of 1..5 sources (0x820B4950: x 6 m) and the unordered layout's
/// (0x820B49B8, pairs x, z: x 6 m in the world's axes).
const ROW: [[f32; 5]; 5] = [[0.0; 5], [0.5, -0.5, 0.0, 0.0, 0.0], [0.0, 1.0, -1.0, 0.0, 0.0], [0.5, -0.5, 1.5, -1.5, 0.0], [0.0, 1.0, -1.0, 2.0, -2.0]];
const CLUSTER: [[[f32; 2]; 5]; 5] = [
    [[0.0, 0.0], [0.0; 2], [0.0; 2], [0.0; 2], [0.0; 2]],
    [[0.5, -0.5], [-0.5, 0.5], [0.0; 2], [0.0; 2], [0.0; 2]],
    [[1.0, 0.0], [0.707, 0.707], [-0.707, 0.707], [0.0; 2], [0.0; 2]],
    [[1.1, 1.1], [1.1, -1.1], [-1.1, -1.1], [-1.1, 1.1], [0.0; 2]],
    [[1.4, 1.4], [1.4, -1.4], [-1.4, -1.4], [-1.4, 1.4], [0.0, 0.0]],
];

/// The third the layout divides by (0x82007FB0, 0x3EAAAAAB).
const THIRD: f32 = 0.33333334;

/// A checkpoint as the layout reads it: its position (+64 / +112), the line's other end if it
/// is a line checkpoint (+80, +330), its activation radius (+168) and height (+176).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CpIn {
    pub pos: Vec3,
    pub line_end: Option<Vec3>,
    pub radius: f32,
    pub height: f32,
}

/// `PickUpManager_CreatePowerUpSourcesFromCheckPoints(list, ordered)` (sub_826830A0). For
/// each checkpoint (the last one takes the one before it - and, ordered, no direction):
/// a line checkpoint makes int((length - 5) / 3) sources (5 is taken off from 5 m), along the
/// line; a round one int((radius - 6) / 3) (3 taken off, and 3 more from 6 m), across the way
/// to the next checkpoint (ordered) or in a cluster (unordered); 1..5, and an even number when
/// ordered. 6 m apart, probed 1 m up and down from the checkpoint's height.
pub fn checkpoint_spots(cps: &[CpIn], ordered: bool) -> Vec<Spot> {
    let n = cps.len();
    let mut out = Vec::new();
    let mut dir = Vec3::ZERO;
    for k in 0..n {
        let last = k + 1 == n;
        let rec = if last && n >= 2 { cps[n - 2] } else { cps[k] };
        let mut count;
        if let Some(b) = rec.line_end {
            let mut l = (rec.pos - b).length();
            if l >= 5.0 {
                l -= 5.0;
            }
            count = ((l * THIRD) as i32).clamp(1, 5);
            let d = b - rec.pos;
            let r = Vec3::new(d.z, 0.0, -d.x);
            dir = if r.length_squared() > 0.0 { r.normalize() } else { Vec3::ZERO };
        } else {
            let mut f = rec.radius - 3.0;
            if f >= 3.0 {
                f -= 3.0;
            }
            count = ((f * THIRD) as i32).clamp(1, 5);
            if ordered {
                let next = if last && n >= 2 { cps[n - 2].pos } else { cps.get(k + 1).map_or(rec.pos, |c| c.pos) };
                let d = next - rec.pos;
                let d = Vec3::new(d.x, 0.0, d.z);
                dir = if d.length_squared() > 0.0 { d.normalize() } else { Vec3::ZERO };
            }
        }
        if count % 2 == 1 && ordered {
            count -= 1;
        }
        for i in 0..count.max(0) as usize {
            let mut p = rec.pos;
            if !ordered && rec.line_end.is_none() {
                p.x += CLUSTER[count as usize - 1][i][0] * 6.0;
                p.z += CLUSTER[count as usize - 1][i][1] * 6.0;
            } else {
                let t = ROW[count as usize - 1][i];
                p.x += t * dir.z * 6.0;
                p.z -= t * dir.x * 6.0;
            }
            out.push(Spot { at: p, up: 1.0, down: 1.0, height: rec.height, last });
        }
    }
    out
}

/// The race script's own list: what `PickUpManager_AddPowerUpSource` is given in its loop
/// (position, width, second point), read by running the script's stores of constants into its
/// local arrays up to that call. None: the script has no list (the core then lays the sources
/// out from the checkpoints).
pub fn script_sources(sco: &mcla_formats::sco::Sco) -> Option<Vec<(Vec3, f32, Vec3)>> {
    use std::collections::HashMap;
    const ADD: u32 = 0x5023685e;
    const CREATE: u32 = 0x8a115016;
    let count = sco.calls().into_iter().find(|c| c.hash == CREATE).and_then(|c| c.args.first().and_then(|a| a.num()))? as usize;
    let code = sco.code();
    let n = code.len();
    let u16le = |p: usize| if p + 2 <= n { u16::from_le_bytes([code[p], code[p + 1]]) } else { 0 };
    let u32le = |p: usize| if p + 4 <= n { u32::from_le_bytes([code[p], code[p + 1], code[p + 2], code[p + 3]]) } else { 0 };
    let len = |pc: usize| -> usize {
        let at = |i: usize| code.get(pc + i).copied().unwrap_or(0) as usize;
        match code[pc] {
            40 => 3,
            41 | 42 | 34 | 35 | 36 | 46 => 5,
            45 => 7,
            47 => 4,
            48 => 3,
            66 => 2 + at(1) * 8,
            67 => 2 + at(1),
            69..=72 => 2,
            _ => 1,
        }
    };
    #[derive(Clone, Copy, PartialEq)]
    enum V {
        N(f64),
        A(i64),
        U,
    }
    let mut stack: Vec<V> = Vec::new();
    let mut mem: HashMap<i64, f64> = HashMap::new();
    let mut recent: Vec<(u8, i64)> = Vec::new();
    let mut pc = 0;
    let mut arrays: Option<Vec<(i64, i64)>> = None;
    while pc < n {
        let op = code[pc];
        let a = pc + 1;
        let pop = |s: &mut Vec<V>| s.pop().unwrap_or(V::U);
        let imm = match op {
            80..=255 => Some(op as i64 - 96),
            40 => Some(u16le(a) as i16 as i64),
            41 => Some(u32le(a) as i32 as i64),
            _ => None,
        };
        recent.push((op, imm.unwrap_or(0)));
        if recent.len() > 64 {
            recent.remove(0);
        }
        match op {
            80..=255 | 40 | 41 => stack.push(V::N(imm.unwrap_or(0) as f64)),
            42 => stack.push(V::N(f32::from_bits(u32le(a)) as f64)),
            37 => {}
            7 | 19 => {
                let v = pop(&mut stack);
                stack.push(if let V::N(x) = v { V::N(-x) } else { V::U });
            }
            54..=61 => stack.push(V::A(op as i64 - 54)),
            62 => {
                let v = pop(&mut stack);
                stack.push(if let V::N(x) = v { V::A(x as i64) } else { V::U });
            }
            65 => {
                let arr = pop(&mut stack);
                let size = pop(&mut stack);
                let idx = pop(&mut stack);
                stack.push(match (arr, size, idx) {
                    (V::A(b), V::N(s), V::N(i)) => V::A(b + 1 + i as i64 * s as i64),
                    _ => V::U,
                });
            }
            50 => {
                let addr = pop(&mut stack);
                let v = pop(&mut stack);
                if let (V::A(p), V::N(x)) = (addr, v) {
                    mem.insert(p, x);
                }
            }
            51 => {
                let v = pop(&mut stack);
                if let (Some(V::A(p)), V::N(x)) = (stack.last().copied(), v) {
                    mem.insert(p, x);
                }
            }
            53 => {
                let addr = pop(&mut stack);
                let cnt = match pop(&mut stack) {
                    V::N(c) if (0.0..64.0).contains(&c) => c as usize,
                    _ => 0,
                };
                let mut vals: Vec<V> = (0..cnt).map(|_| pop(&mut stack)).collect();
                vals.reverse();
                if let V::A(p) = addr {
                    for (j, v) in vals.into_iter().enumerate() {
                        if let V::N(x) = v {
                            mem.insert(p + j as i64, x);
                        }
                    }
                }
            }
            44 => {
                pop(&mut stack);
            }
            45 if u32le(a + 2) == ADD => {
                // the arrays the call indexes: `push size; push base; local; array` or
                // `push size; localN; array`, in the order of its arguments
                let mut found = Vec::new();
                let r = &recent;
                for j in 0..r.len() {
                    if r[j].0 != 65 {
                        continue;
                    }
                    if j >= 3 && r[j - 1].0 == 62 && matches!(r[j - 2].0, 40 | 41 | 80..=255) && matches!(r[j - 3].0, 40 | 41 | 80..=255) {
                        found.push((r[j - 2].1, r[j - 3].1));
                    } else if j >= 2 && (54..=61).contains(&r[j - 1].0) && matches!(r[j - 2].0, 40 | 41 | 80..=255) {
                        found.push((r[j - 1].0 as i64 - 54, r[j - 2].1));
                    }
                }
                arrays = Some(found);
                break;
            }
            _ => stack.clear(),
        }
        pc += len(pc);
    }
    let arrays = arrays?;
    if arrays.len() < 3 {
        return None;
    }
    let (p, w, q) = (arrays[arrays.len() - 3], arrays[arrays.len() - 2], arrays[arrays.len() - 1]);
    if p.1 != 3 || w.1 != 1 || q.1 != 3 {
        return None;
    }
    let mut out = Vec::new();
    for i in 0..count as i64 {
        let v3 = |b: i64| -> Option<Vec3> { Some(Vec3::new(*mem.get(&(b + 1 + i * 3))? as f32, *mem.get(&(b + 2 + i * 3))? as f32, *mem.get(&(b + 3 + i * 3))? as f32)) };
        let (Some(pos), Some(width), Some(second)) = (v3(p.0), mem.get(&(w.0 + 1 + i)).copied(), v3(q.0)) else { break };
        out.push((pos, width as f32, second));
    }
    (!out.is_empty()).then_some(out)
}

/// The race script's list for the race with this id ("game/Race/Downtown/Circuit/dt_circuit_mb_01").
pub fn script_list(data_dir: &str, race_id: &str) -> Option<Vec<(Vec3, f32, Vec3)>> {
    let rel = race_id.to_ascii_lowercase();
    let (folder, name) = rel.rsplit_once('/')?;
    let dir = std::path::Path::new(data_dir).join("script_obj").join(folder);
    let p = dir.join(format!("{name}.sco"));
    let p = if p.is_file() { p } else { dir.join(format!("0x{:08x}", mcla_formats::hash::joaat(&format!("{name}.sco")))) };
    let sco = mcla_formats::sco::decode(&std::fs::read(p).ok()?).ok()?;
    script_sources(&sco)
}

/// The race's checkpoints of one lap as the layout reads them (the port's race: a gate's ends,
/// the radii - 24 m where the script gave none -; the height is the checkpoint's default, 100,
/// sub_82266208: the scripts' own heights are not read by the port).
pub fn race_checkpoints(r: &crate::game::Race) -> Vec<CpIn> {
    let n = if r.lap_len > 0 { r.lap_len.min(r.checkpoints.len()) } else { r.checkpoints.len() };
    (0..n)
        .map(|k| {
            let gate = r.cp_gate.get(k).copied().flatten();
            CpIn { pos: gate.map_or(r.checkpoints[k], |g| g.0), line_end: gate.map(|g| g.1), radius: r.cp_radius.get(k).copied().unwrap_or(24.0), height: 100.0 }
        })
        .collect()
}

/// A pick-up source (mcPowerUpSource, 160 bytes).
#[derive(Clone, Debug)]
pub struct Source {
    pub spot: Spot,
    /// where the probe found the ground (None: not yet probed; the probe missed: `dead`)
    pub pos: Option<Vec3>,
    pub dead: bool,
    /// +136 the type it gives, +144 shown as the Mystery one
    pub kind: u8,
    pub mystery: bool,
    /// +140: 0 waiting to come back, 1 there, 3 never again
    pub state: u8,
    /// +148 the time it comes back
    pub back_at: f32,
}

/// The types a source may give (+128) and whether it may be a Mystery one (+22), from the
/// options record (sub_82682770): byte 0 bits 7..0 Nitro .. Pulse, byte 1 bits 7, 6 Mirror,
/// Agro, bit 5 RANDOM.
pub fn allowed(record: &[u8; 28]) -> (u32, bool) {
    let mut bits = 0u32;
    for t in 0..8 {
        if record[0] & (0x80 >> t) != 0 {
            bits |= 1 << t;
        }
    }
    if record[1] & 0x80 != 0 {
        bits |= 1 << MIRROR;
    }
    if record[1] & 0x40 != 0 {
        bits |= 1 << AGRO;
    }
    (bits, record[1] & 0x20 != 0)
}

/// A source's type from a random number (sub_82683A28): with RANDOM allowed, one in four
/// (the number's low two bits 0) is shown as Mystery; the type is the (number >> 2) mod
/// count-th allowed one, in type order (sub_82683928); none allowed: 25. Nitro is never a
/// Mystery one.
pub fn roll(bits: u32, mystery_ok: bool, r: i32, was_mystery: bool) -> (u8, bool) {
    let mut mystery = was_mystery;
    if mystery_ok {
        mystery = r & 3 == 0;
    }
    let count = bits.count_ones() as i32;
    if count == 0 {
        return (NONE, mystery);
    }
    let idx = (r >> 2) % count;
    let kind = (0..32).filter(|t| bits & (1 << t) != 0).nth(idx as usize).unwrap_or(0) as u8;
    if mystery && kind == NITRO {
        mystery = false;
    }
    (kind, mystery)
}

// ---------------------------------------------------------------------------------
// The car's holder

/// A shot looking for its target (+228 with +232 set): the type, the time left (+16), the
/// targets found (+44.., +104) and the time since (+108).
#[derive(Clone, Debug, PartialEq)]
pub struct Shot {
    pub kind: u8,
    pub left: f32,
    pub found: bool,
    pub targets: Vec<usize>,
    pub wait: f32,
}

/// An effect on the car (+236) and its time left (+12).
#[derive(Clone, Debug, PartialEq)]
pub struct Effect {
    pub kind: u8,
    pub left: f32,
    melted: bool,
}

/// What a car holds (mcCarPickups at the racer +68).
#[derive(Clone, Debug)]
pub struct Holder {
    pub car: usize,
    pub slots: [u8; 2],
    pub shot: Option<Shot>,
    pub effect: Option<Effect>,
    /// +112 the shot was fired backwards (LOOKBACK held)
    pub back: bool,
    /// the vehicle's control override (+624: 1/2 brake off/on, 4/8 throttle off/on,
    /// 0x10/0x20 steer full left/right, 0x40 steering reversed, 0x80 ice, 0x100/0x200 handbrake
    /// off/on, 0x400 steer centred, 0x800 throttle off, 0x1000 steering reversed)
    pub ctl: u32,
    /// Shield: hits taken (the type's +104)
    pub shield_hits: u32,
    /// Agro: the ability the car had (the type's +104)
    pub agro_prev: Option<crate::ability::Kind>,
    /// where the car was a frame ago (the racer's +400 as the source's test reads it: the
    /// position row of the racer's second matrix, +352..+415, which sub_822A9930 refreshes from
    /// its matrix at +288)
    pub prev: Option<Vec3>,
    /// step 19f: the racer has finished (sub_8232A820: racer +36 >= (racer +8) +24) - it takes
    /// nothing more
    pub finished: bool,
    /// step 19f: the vehicle lets it fire (sub_8232A728 -> the vehicle's slot 12, sub_8232C368:
    /// chassis +592 == 0, or on a motorcycle exactly 0x10 - the rider off)
    pub veh_ok: bool,
    /// step 19f: a wasted shot's icon (+164 type, +176 where, +192 the second it shows for,
    /// sub_8232A4B0; counted down by 1/60 a tick, sub_8232B150 0x8232B34C)
    pub icon: Option<(u8, Vec3, f32)>,
    /// step 19g: what the effect's draw reads (pu_fx.rs)
    pub fx: CarFx,
    /// step 19g: the lightning of Reverse / Stop / Disrupter (pu_bolt.rs)
    pub bolts: Option<Box<crate::pu_bolt::Lightning>>,
    /// step 19g: the shot's laser fans (the shoot-effect manager's slots, sub_82747650): the
    /// race time it started, the time it ends (start + ShootTime; set to "now" when the shot
    /// lands or is wasted - sub_82747720), backwards, the type
    pub lasers: Vec<Laser>,
}

/// Step 19g: one laser fan of a shot (a slot of the manager [*0x8288B9AC + 52], +880 + 64 i:
/// +912 start, +916 end, +921 backwards, +928 the type's look; drawn by sub_82747E20 until
/// 0.25 s (+1400) after its end - sub_827481C0).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Laser {
    pub start: f32,
    pub end: f32,
    pub back: bool,
    pub kind: u8,
}

/// Step 19g: the timers the on-car effects are drawn with. The original keeps them in the TYPE
/// object (one per type, shared by every car with that effect - offline only the player can
/// have a Shield); here per car.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CarFx {
    /// Shield +376: the hit flash's clock (1 = no flash; set to 0 by a shot taken, sub_8237E1F8;
    /// + 1/60 x +380 (1) a frame up to 1, sub_8237FB18) - the key curve's time for FX_Timer1
    pub flash: f32,
    /// Shield +384: FX_Timer2, + 1/60 x +368 (0.18) a frame from 0 at the start (sub_8237E218)
    pub t2: f32,
    /// Shield +388: the clock of FX_Timer3 - every +372 (0.24 s) +392 takes a new random number
    pub t3_clock: f32,
    /// Shield +392: FX_Timer3 (a random number in [0, 1))
    pub t3: f32,
    /// a random state for FX_Timer3 (the original's generator 0x82860880 is shared by the game)
    pub rng: u32,
}

impl Default for CarFx {
    fn default() -> Self {
        CarFx { flash: 1.0, t2: 0.0, t3_clock: 0.0, t3: 0.0, rng: 0x1234_5678 }
    }
}

impl CarFx {
    /// The Shield's per-frame update (sub_8237FB18; 1/60 = the clock object's +8, 0x827D7508):
    /// HitByAgro's (sub_82380670) is the same with the flash held at 1.
    pub fn shield_tick(&mut self, dt: f32) {
        self.flash = (self.flash + 1.0 * dt).min(1.0);
        self.t2 += 0.18 * dt;
        self.t3_clock += dt;
        if self.t3_clock > 0.24 {
            // (two numbers are drawn, +392 and +396; +396 is read by nothing)
            self.t3 = self.next();
            let _ = self.next();
            self.t3_clock = 0.0;
        }
    }

    fn next(&mut self) -> f32 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        (x & 0x7f_ffff) as f32 * 1.192_092_9e-7
    }
}

impl Holder {
    pub fn new(car: usize) -> Holder {
        Holder { car, slots: [NONE; 2], shot: None, effect: None, back: false, ctl: 0, shield_hits: 0, agro_prev: None, prev: None, finished: false, veh_ok: true, icon: None, bolts: None, lasers: Vec::new(), fx: CarFx { rng: (0x9e37_79b9 ^ (car as u32).wrapping_mul(0x85eb_ca6b)) | 1, ..Default::default() } }
    }

    /// Two megas in hand (the manager's test, sub_82680B68): slot 1 not empty and both >= 11.
    pub fn megas(&self) -> bool {
        self.slots[1] != NONE && self.slots[1] >= 11 && self.slots[0] >= 11
    }

    /// May this power-up be taken (sub_8232A820)? `shots`: the nitrous shots in the car.
    pub fn accept(&self, kind: u8, shots: u8, types: &[TypeTune; 24]) -> bool {
        // (the racer has finished: nothing)
        if self.finished || kind == NONE {
            return false;
        }
        if kind == NITRO {
            return shots < 5;
        }
        if self.slots[0] == NONE || self.slots[1] == NONE {
            return true;
        }
        (kind == self.slots[0] || kind == self.slots[1]) && types.get(kind as usize).map_or(false, |t| t.mega != NONE)
    }

    /// Take it (sub_8232B540): Nitro is one more nitrous shot (up to 5); the same one again
    /// with the second slot empty makes its mega, which fills both; else the first empty
    /// slot. Returns the type taken (None: not taken).
    pub fn give(&mut self, kind: u8, shots: &mut u8, types: &[TypeTune; 24]) -> Option<u8> {
        if kind == NITRO {
            if *shots >= 5 {
                return None;
            }
            *shots += 1;
            return Some(NITRO);
        }
        if kind == NONE {
            return None;
        }
        if kind == self.slots[0] && self.slots[1] == NONE {
            let m = types[kind as usize].mega;
            self.slots = [m, m];
            return Some(m);
        }
        if self.slots[0] == NONE {
            self.slots[0] = kind;
        } else if self.slots[1] == NONE {
            self.slots[1] = kind;
        } else {
            return None;
        }
        Some(kind)
    }

    /// May the car fire (sub_8232A728): something in hand, no shot out, no effect on it, and the
    /// vehicle lets it (its slot 12, sub_8232C368 - `veh_ok`).
    pub fn can_fire(&self) -> bool {
        self.slots[0] != NONE && self.shot.is_none() && self.effect.is_none() && self.veh_ok
    }

    /// The slots after firing (sub_8232BB90): a mega goes at once.
    fn shift(&mut self) {
        if self.slots[0] == self.slots[1] {
            self.slots = [NONE; 2];
        } else {
            self.slots = [self.slots[1], NONE];
        }
    }
}

/// A PowerUptype cell's word (F-56191): the type + 1, none (25) 0
pub fn hud_cell(t: i32) -> i32 {
    if t == NONE as i32 { 0 } else { t + 1 }
}

/// The HUD's power-up control (sub_82203AC8 -> sub_8223E200): `PowerUptype_0`,
/// `PowerUptype_1` (the type + 1, 0 empty; a mega shows as its base) and `PowerUpvalue`.
/// With a shot out: its type - less 11 when under 22, as the original does it - then the
/// first slot, and the shot's time left over the type's ActiveTime.
pub fn hud_values(h: &Holder, types: &[TypeTune; 24]) -> [f32; 3] {
    let map = |t: u8| -> i32 {
        let t = t as i32;
        hud_cell(if (11..22).contains(&t) { t - 11 } else { t })
    };
    match &h.shot {
        Some(s) => {
            let mut a = s.kind as i32;
            if a < 22 {
                a -= 11;
            }
            let a = hud_cell(a);
            [a as f32, map(h.slots[0]) as f32, s.left / types[s.kind as usize].active_time.max(1e-3)]
        }
        None => [map(h.slots[0]) as f32, map(h.slots[1]) as f32, 1.0],
    }
}

/// The control override on a car's input (sub_8232EA78 0x8232EB18..0x8232EC28; the ice bit
/// is read by the wheels, 0x823300EC: their handling factor 10 - the grip a tenth).
pub fn apply_ctl(input: &mut crate::physics::car::CarInput, ctl: u32) {
    if ctl == 0 {
        return;
    }
    if ctl & 0x1 != 0 {
        input.brake = 0.0;
    } else if ctl & 0x2 != 0 {
        input.brake = 1.0;
    }
    if ctl & 0x100 != 0 {
        input.handbrake = false;
    } else if ctl & 0x200 != 0 {
        input.handbrake = true;
    }
    if ctl & 0x8 != 0 {
        input.throttle = 1.0;
    } else if ctl & 0x4 != 0 {
        input.throttle = 0.0;
    }
    if ctl & 0x400 != 0 {
        input.steer = 0.0;
    } else if ctl & 0x10 != 0 {
        input.steer = -1.0;
    } else if ctl & 0x20 != 0 {
        input.steer = 1.0;
    } else if ctl & 0x40 != 0 {
        input.steer = -input.steer;
    }
    if ctl & 0x800 != 0 {
        input.throttle = 0.0;
    } else if ctl & 0x1000 != 0 {
        input.steer = -input.steer;
    }
    if ctl & 0x80 != 0 {
        input.ice = true;
    }
}

/// The physics tick (1/dt = 59.999996 in the original).
pub const TICK: f32 = 1.0 / 60.0;

/// Pulse's push (sub_82380418, step 19f), world axes: the force per unit of mass and the torque
/// per unit of the body's own (diagonal, car-axes) inertia. The instance's rows A (+16, right),
/// C (+48, back): sx = sign(A . ForceDir), s4 = sign(-(C . ForceDir)) x sx - ForceDir taken in
/// WORLD axes, as the code does it; force = rows x (Force.x sx, Force.y, Force.z), torque =
/// rows x (Torque x s4). The routine then multiplies the force by the archetype's mass (its
/// slot 7) and the world torque by the archetype's inertia vector (slot 9) component by
/// component, and hands both to the simulator (sub_8258D8B8 / sub_8258D938) - for a car
/// (instance vtable 0x82045F44, slot 8 = 0) the collider's slot 31 (0x8259E0F0: force at the
/// bound's centroid, into +336 / +352) and slot 33 (0x8259C0F0: torque into +352). Those are
/// the force and torque the next tick integrates over its dt and clears (0x8259BE30): the
/// change of speed is Force / 60 - (2.5, 5, 0) m/s in the car's frame for Pulse.
pub fn pulse_push(rot: Quat, t: &TypeTune) -> (Vec3, Vec3) {
    let sgn = |v: f32| if v > 0.0 { 1.0 } else if v < 0.0 { -1.0 } else { 0.0 };
    let sx = sgn((rot * Vec3::X).dot(t.force_dir));
    let s4 = sgn(-(rot * Vec3::Z).dot(t.force_dir)) * sx;
    (rot * Vec3::new(t.force.x * sx, t.force.y, t.force.z), rot * (t.torque * s4))
}

/// The vehicle's own "may it fire" (its slot 12, sub_8232C368): chassis +592 nothing, or - on
/// a two-wheeler - exactly 0x10 (the rider off). The port keeps only the motorcycle's 0x10
/// (`VehState::wipe`); a car's other bits of +592 are not modelled, so a car always may.
pub fn vehicle_lets_fire(flags592: u32, wheels: u32) -> bool {
    flags592 == 0 || (wheels == 2 && flags592 == 0x10)
}

/// The types the sources get (sub_82682770 reads them from the session's options copy,
/// (*0x8286D804 +48) +0x2ADE0 - step 19f): that copy is written only by three network
/// message handlers (sub_822837B8 and sub_82289F20 through EventOptions' defaults sub_823974A8,
/// sub_82282EB8 through sub_8227D1B0), each with byte 0 = 0xFF and byte 1 | 0xE0 - all ten
/// types and RANDOM. Nothing copies the ARCADE menu's record into it. MCLA_PU_RECORD_TYPES=1:
/// the ARCADE record's bits instead (the step 19e reading).
pub const SESSION_TYPES: [u8; 2] = [0xff, 0xe0];

/// How a target ranks for a shot (sub_8232B718): a car under Stealth 1e6; with a negative
/// width the distance; else the target in the shooter's frame - out of the box (|x| or |y|
/// over the width) 1e6, in it the distance ahead.
pub fn rank(shooter_pos: Vec3, shooter_rot: Quat, target_pos: Vec3, width: f32, stealth: bool) -> f32 {
    if stealth {
        return 1e6;
    }
    if width < 0.0 {
        return (target_pos - shooter_pos).length();
    }
    let l = shooter_rot.inverse() * (target_pos - shooter_pos);
    if l.x.abs() > width || l.y.abs() > width {
        return 1e6;
    }
    -l.z
}

/// The targets of a shot (sub_8232BCE8): every other racer whose rank is between 0 and the
/// range - backwards (LOOKBACK) the rank turned round, but not for a mega -, the nearest one,
/// or all of them for a mega (types 12..20).
pub fn targets(cars: &[(usize, Vec3, Quat, bool)], shooter: usize, kind: u8, back: bool, t: &TypeTune) -> Vec<usize> {
    let mega = kind > 11 && kind < 21;
    let Some(&(_, sp, sr, _)) = cars.iter().find(|c| c.0 == shooter) else { return Vec::new() };
    let mut best = t.shoot_range;
    let mut out = Vec::new();
    let mut near = None;
    for &(i, p, _, stealth) in cars {
        if i == shooter {
            continue;
        }
        let mut d = rank(sp, sr, p, t.width, stealth);
        if back && !mega {
            d = -d;
        }
        if d < best && d > 0.0 {
            if mega {
                out.push(i);
            } else {
                near = Some(i);
                best = d;
            }
        }
    }
    if let Some(i) = near {
        out.push(i);
    }
    out
}

// ---------------------------------------------------------------------------------
// The sounds (game.dat PACKAGE_POWERUP_PLAYER: POWERUP_<X>_PLAYER per type; the computer
// racers' package has none). Fields of the object as the car audio's notifications play them
// (sub_823497D8 .. sub_82349EC8): +10 picked up, +14 a hit landed, +18 / +22 fired (and its
// loop), +30 / +34 in (and its loop), +38 out, +42 mirrored, +46 shielded, +54 melting.
// The first wave of each object (sounds.dat), left channel.

const BANK: &str = "special_effects/bnk_special_effects/";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Snd {
    Pickup,
    Hit,
    Fire,
    In,
    Out,
    Mirrored,
    Shielded,
    Melt,
}

/// The wave of a notification for a type (None: NULL_SOUND).
pub fn wave(kind: u8, s: Snd) -> Option<String> {
    let base = if (12..21).contains(&kind) { kind - 11 } else { kind };
    if base > AGRO {
        return None;
    }
    let w: Option<&str> = match s {
        Snd::Pickup => Some(["powerups_pickups_nitro", "powerups_pickups_ice_left", "powerups_pickups_reverse_left", "powerups_pickups_stealth_left", "powerup_disruptor_pickup.l", "powerups_pickups_shield_left", "powerups_pickups_stop_left", "powerups_pickups_pulse_left", "powerups_pickups_mirror_left", "powerup_agro_pickup.l"][base as usize]),
        Snd::Hit => Some("electric_bolt_fire.l"),
        Snd::Fire => Some("8ed3f2e7"),
        Snd::In => [None, Some("powerups_mirror_in_left"), Some("powerups_reverse_in_left"), Some("powerups_mirror_in_left"), Some("disruptor_in.l"), Some("powerups_stop_in_left"), Some("powerups_stop_in_left"), Some("powerups_pulse_in_left"), Some("powerups_mirror_in_left"), None][base as usize],
        Snd::Out => [None, Some("powerups_mirror_out_left"), Some("powerups_mirror_out_left"), Some("powerups_mirror_out_left"), Some("disruptor_out.l"), Some("powerups_reverse_out_left"), None, None, Some("powerups_mirror_out_left"), None][base as usize],
        Snd::Mirrored => matches!(base, ICE | MIRROR).then_some("electric_bolt_fire.l"),
        Snd::Shielded => None,
        Snd::Melt => (base == ICE).then_some("ice_melt_left"),
    };
    w.map(|w| format!("{BANK}{w}"))
}

/// The player's power-up sound as the original names it: game.dat `POWERUP_<X>_PLAYER` (PACKAGE_POWERUP_PLAYER) and the
/// field of the event (+10 picked up, +14 hit, +18 fired, +30 in, +38 out, +42 mirrored, +46 shielded, +54 melting);
/// +50 is the effect's dynamic mixer (on at In, off at Out).
pub fn object(kind: u8, s: Snd) -> Option<(String, usize)> {
    let base = if (12..21).contains(&kind) { kind - 11 } else { kind };
    let name = ["NITRO", "ICE", "REVERSE", "STEALTH", "DISRUPTOR", "SHIELD", "STOP", "PULSE", "MIRROR", "AGRO"].get(base as usize)?;
    let field = match s {
        Snd::Pickup => 10,
        Snd::Hit => 14,
        Snd::Fire => 18,
        Snd::In => 30,
        Snd::Out => 38,
        Snd::Mirrored => 42,
        Snd::Shielded => 46,
        Snd::Melt => 54,
    };
    Some((format!("POWERUP_{name}_PLAYER"), field))
}

/// The loops: the shot's (+22) and the effect's "static" one (+34).
pub fn loop_wave(kind: u8, shot: bool) -> Option<String> {
    let base = if (12..21).contains(&kind) { kind - 11 } else { kind };
    if base > AGRO {
        return None;
    }
    if shot {
        return Some(format!("{BANK}e1f714a0"));
    }
    matches!(base, REVERSE | STOP).then(|| format!("{BANK}powerup_pulse_static.l"))
}

// ---------------------------------------------------------------------------------
// The manager

/// What happened this frame that the game shows or plays.
#[derive(Clone, Debug, PartialEq)]
pub struct Note {
    pub car: usize,
    pub kind: u8,
    pub snd: Snd,
}

pub struct PowerUps {
    pub types: [TypeTune; 24],
    pub looks: [Look; 24],
    pub sources: Vec<Source>,
    pub holders: Vec<Holder>,
    bits: u32,
    mystery_ok: bool,
    rng: u32,
    pub time: f32,
    pub notes: Vec<Note>,
    /// step 19g: the lightning's settings - the manager's bolts and the types' ma_ / mi_ arcs
    /// (Reverse 2, Disrupter 4, Stop 6 and their megas)
    pub bolt_tunes: Vec<(u8, crate::pu_bolt::BoltTune, [crate::pu_bolt::ArcTune; 2])>,
    /// step 19g: the landings of the last 2 s (the manager's target records, sub_82748340: the
    /// rmptfx effect "powerup_sparks" at the target, (0, 0.5, 0) along its axes, with its
    /// velocity, tinted m_beamColor of the type's look; dropped 2 s after - sub_827481C0)
    pub hits: Vec<Hit>,
}

/// Step 19g: one landing (a target record of the shoot-effect manager).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    pub from: usize,
    pub to: usize,
    pub kind: u8,
    pub start: f32,
    /// a number to tell the records apart
    pub key: u64,
}

impl PowerUps {
    /// The manager of a race: the types allowed by the options record, the sources at their
    /// spots (each given its type by OnRaceReset: one random number, source i taking it + i),
    /// a holder for every racer.
    pub fn new(data_dir: &str, record: &[u8; 28], spots: Vec<Spot>, racers: &[usize], seed: u32) -> PowerUps {
        // (step 19f: the session copy's bits, `SESSION_TYPES`; the menu's record only on request)
        let mut session = [0u8; 28];
        session[0] = SESSION_TYPES[0];
        session[1] = SESSION_TYPES[1];
        let (bits, mystery_ok) = allowed(if std::env::var("MCLA_PU_RECORD_TYPES").map_or(false, |v| v == "1") { record } else { &session });
        let mut p = PowerUps {
            types: load_types(data_dir),
            looks: load_looks(data_dir),
            sources: spots.into_iter().map(|spot| Source { spot, pos: None, dead: false, kind: NONE, mystery: false, state: 1, back_at: 0.0 }).collect(),
            holders: racers.iter().map(|&c| Holder::new(c)).collect(),
            bits,
            mystery_ok,
            rng: seed | 1,
            time: 0.0,
            notes: Vec::new(),
            bolt_tunes: load_bolt_tunes(data_dir),
            hits: Vec::new(),
        };
        p.reset();
        p
    }

    fn rand(&mut self) -> i32 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        (x & 0x7fff_ffff) as i32
    }

    /// PickUpManager_OnRaceReset (sub_82680328 -> sub_82683B28).
    pub fn reset(&mut self) {
        let r = self.rand();
        let (bits, ok) = (self.bits, self.mystery_ok);
        for (i, s) in self.sources.iter_mut().enumerate() {
            let (k, m) = roll(bits, ok, r.wrapping_add(i as i32), s.mystery);
            s.kind = k;
            s.mystery = m;
            s.state = 1;
            s.back_at = self.time + 2.0;
        }
    }

    pub fn holder(&self, car: usize) -> Option<&Holder> {
        self.holders.iter().find(|h| h.car == car)
    }

    /// The HUD's three values for a car (no holder: empty slots).
    pub fn hud(&self, car: usize) -> [f32; 3] {
        self.holder(car).map_or([0.0, 0.0, 1.0], |h| hud_values(h, &self.types))
    }

    /// IN_ZONE pressed (sub_822A0398 -> sub_8232BC98): with something to fire, the first
    /// slot is fired - backwards while LOOKBACK is held - and the press does not reach the
    /// special ability. Returns whether it was taken.
    pub fn fire(&mut self, car: usize, back: bool, vehicles: &mut crate::vehicles::Vehicles, ability: &mut crate::ability::Abilities) -> bool {
        let Some(hi) = self.holders.iter().position(|h| h.car == car) else { return false };
        if !self.holders[hi].can_fire() {
            return false;
        }
        let kind = self.holders[hi].slots[0];
        let t = self.types[kind as usize];
        if t.shootable {
            // (sub_8232BB90: the shot, ShootTime to find its target - sub_8237EBD0 looks at once)
            self.holders[hi].back = back;
            let cars = self.racer_cars(vehicles);
            let found = if no_target_hook() { Vec::new() } else { targets(&cars, car, kind, back, &t) };
            self.holders[hi].shot = Some(Shot { kind, left: t.shoot_time, found: !found.is_empty(), targets: found, wait: 0.0 });
            self.notes.push(Note { car, kind, snd: Snd::Fire });
            // (step 19g, sub_8237C718: the laser - a mega shoots two, backwards and forwards;
            // the others one, backwards when LOOKBACK is held)
            let now = self.time;
            let mk = |b: bool| Laser { start: now, end: now + t.shoot_time, back: b, kind };
            self.holders[hi].lasers.retain(|l| now - l.start <= (l.end - l.start) + LASER_OUT);
            if (12..21).contains(&kind) {
                self.holders[hi].lasers.push(mk(true));
                self.holders[hi].lasers.push(mk(false));
            } else {
                self.holders[hi].lasers.push(mk(back));
            }
        } else {
            // (sub_8232B4B8 -> sub_8232A428: on the own car)
            self.apply(hi, kind, vehicles, ability);
        }
        self.holders[hi].shift();
        true
    }

    fn racer_cars(&self, vehicles: &crate::vehicles::Vehicles) -> Vec<(usize, Vec3, Quat, bool)> {
        self.holders
            .iter()
            .filter_map(|h| vehicles.cars.get(h.car).map(|c| (h.car, c.state.pos, c.state.rot, h.effect.as_ref().map_or(false, |e| e.kind == STEALTH))))
            .collect()
    }

    /// An effect starts on a car (sub_8232A428: the type's +12, then +236 and ActiveTime).
    fn apply(&mut self, hi: usize, kind: u8, vehicles: &mut crate::vehicles::Vehicles, ability: &mut crate::ability::Abilities) {
        let car = self.holders[hi].car;
        let t = self.types[kind as usize];
        let base = if (12..21).contains(&kind) { kind - 11 } else { kind };
        match base {
            // (sub_8237D020: +624 = 128; sub_8237D110: 64 and the lightning; sub_8237E978: 6)
            ICE => self.holders[hi].ctl = 0x80,
            REVERSE => self.holders[hi].ctl = 0x40,
            STOP => self.holders[hi].ctl = 6,
            // (sub_8237E218: the shield shown, its hits counted from 0; step 19g: its timers
            // +384 .. +396 from 0 - the flash clock +376 is left as it is)
            SHIELD => {
                self.holders[hi].shield_hits = 0;
                let fx = &mut self.holders[hi].fx;
                fx.t2 = 0.0;
                fx.t3_clock = 0.0;
                fx.t3 = 0.0;
            }
            // (sub_8237E638: the car's ability made Agro, its gauge full and running)
            AGRO => {
                if vehicles.cars.get(car).map_or(false, |c| c.is_player) {
                    self.holders[hi].agro_prev = Some(ability.powerup_run(crate::ability::Kind::Agro));
                }
            }
            // (sub_82380418, step 19f: the push - see `pulse_push`. It goes into the body's force
            // and torque sums for the next physics tick; a car without the ported body takes the
            // same change of speed by hand)
            PULSE => {
                if let Some(c) = vehicles.cars.get_mut(car) {
                    let (pf, pt) = pulse_push(c.state.rot, &t);
                    log::info!("powerups: pulse on car {car}: force {:.0?} per kg, torque {:.0?} per inertia, speed before {:.1?}", pf, pt, c.state.vel);
                    if let Some(vs) = c.state.veh.as_mut() {
                        vs.pulse = Some((pf, pt));
                    } else {
                        c.state.vel += pf * TICK;
                        c.state.ang_vel += pt * TICK;
                    }
                }
            }
            _ => {}
        }
        // (step 19g: the lightning - Reverse (sub_8237D110) and Stop (sub_8237E978) on any car, the
        // Disrupter (sub_8237D920) on any but the local player's, sub_82331D40)
        let player = vehicles.cars.get(car).map_or(false, |c| c.is_player);
        self.holders[hi].bolts = match base {
            REVERSE | STOP => self.bolt_tunes.iter().find(|b| b.0 == kind),
            DISRUPTER if !player => self.bolt_tunes.iter().find(|b| b.0 == kind),
            _ => None,
        }
        .map(|b| Box::new(crate::pu_bolt::Lightning::start(b.1, b.2, (car as u32 + 1).wrapping_mul(0x2545_F491) ^ (self.time.to_bits()))));
        self.holders[hi].effect = Some(Effect { kind, left: t.active_time, melted: false });
        self.notes.push(Note { car, kind, snd: Snd::In });
    }

    /// The effect ends (sub_8232A590: the type's +16).
    fn end_effect(&mut self, hi: usize, ability: &mut crate::ability::Abilities) {
        self.holders[hi].bolts = None;
        let Some(e) = self.holders[hi].effect.take() else { return };
        let base = if (12..21).contains(&e.kind) { e.kind - 11 } else { e.kind };
        match base {
            ICE | REVERSE | STOP => self.holders[hi].ctl = 0,
            SHIELD => self.holders[hi].shield_hits = 0,
            AGRO => {
                if let Some(k) = self.holders[hi].agro_prev.take() {
                    ability.powerup_stop(k);
                }
            }
            _ => {}
        }
        let car = self.holders[hi].car;
        self.notes.push(Note { car, kind: e.kind, snd: Snd::Out });
    }

    /// A shot lands (sub_8232C080 for the shooter, sub_8232BED8 for the target): the
    /// shooter's shot ends; a Shield takes it (and breaks at the second), Stealth lets it
    /// pass, a Mirror sends it back (a MegaMirror to every car round it) and ends; else the
    /// target's effect ends and this one starts.
    fn land(&mut self, from: usize, to: usize, kind: u8, vehicles: &mut crate::vehicles::Vehicles, ability: &mut crate::ability::Abilities, depth: u32) {
        let (Some(fi), Some(ti)) = (self.holders.iter().position(|h| h.car == from), self.holders.iter().position(|h| h.car == to)) else { return };
        // (step 19g, sub_8237C5C0 / sub_8232C080: the shooter's lasers stop now, and a target
        // record starts - whatever the target then does with the shot)
        let now = self.time;
        let key = 0x5019_0000_0000_0000u64 | ((self.hits.len() as u64 + (now * 60.0) as u64 * 64) << 8) | to as u64;
        self.hits.push(Hit { from, to, kind, start: now, key });
        for l in self.holders[fi].lasers.iter_mut() {
            l.end = l.end.min(now);
        }
        if self.holders[fi].shot.take().is_some() {
            // (sub_8232A4B0 with a shot: the type's +24, the notification +84 - no sound)
        }
        let cur = self.holders[ti].effect.as_ref().map_or(NONE, |e| e.kind);
        match cur {
            SHIELD | MEGA_SHIELD => {
                self.holders[ti].shield_hits += 1;
                // (sub_8237E1F8: +104 + 1 and the flash clock +376 back to 0)
                self.holders[ti].fx.flash = 0.0;
                self.notes.push(Note { car: to, kind: cur, snd: Snd::Shielded });
                // (the Shield's vfunc +64, sub_8237E0A8: broken from two hits)
                if self.holders[ti].shield_hits >= 2 {
                    self.end_effect(ti, ability);
                }
            }
            STEALTH | MEGA_STEALTH => {}
            MIRROR | MEGA_MIRROR => {
                if depth < 4 {
                    self.land(to, from, kind, vehicles, ability, depth + 1);
                    if cur == MEGA_MIRROR {
                        // (sub_8232BCE8 round the mirroring car with the shot's range and box,
                        // every car found)
                        let t = self.types[kind as usize];
                        let cars = self.racer_cars(vehicles);
                        for j in targets(&cars, to, 12, false, &t) {
                            self.land(to, j, kind, vehicles, ability, depth + 1);
                        }
                    }
                }
                self.end_effect(ti, ability);
                self.notes.push(Note { car: to, kind: cur, snd: Snd::Mirrored });
            }
            _ => {
                self.end_effect(ti, ability);
                self.apply(ti, kind, vehicles, ability);
            }
        }
        self.notes.push(Note { car: from, kind, snd: Snd::Hit });
    }

    /// One frame: sources placed where the ground is there to probe, the cars' shots and
    /// effects (sub_8232B150), the pick-ups (sub_82680D78: the player first, then the
    /// racers), the sources coming back (sub_826836C8). `probe`: the ground under a point
    /// (None while that part of the city is not loaded: Some(None) a miss).
    pub fn update(&mut self, vehicles: &mut crate::vehicles::Vehicles, ability: &mut crate::ability::Abilities, probe: &dyn Fn(Vec3, f32, f32) -> Option<Option<Vec3>>, dt: f32) {
        // (step 19g test hook MCLA_PU_SELF="<type>,<seconds>": that effect starts on the player's
        // OWN car at that race time - as if a shot had landed on it - to look at the on-car
        // effects of the shot types; offline no computer racer fires)
        if let Ok(v) = std::env::var("MCLA_PU_SELF") {
            let mut it = v.split(',');
            let k = it.next().unwrap_or("").trim().to_string();
            let at: f32 = it.next().and_then(|x| x.trim().parse().ok()).unwrap_or(20.0);
            let kind = k.parse::<u8>().ok().or_else(|| NAMES.iter().position(|n| n.eq_ignore_ascii_case(&k)).map(|i| i as u8));
            if let Some(kind) = kind.filter(|&k| (k as usize) < 24) {
                if self.time < at && self.time + dt >= at {
                    if let Some(hi) = self.holders.iter().position(|h| vehicles.cars.get(h.car).map_or(false, |c| c.is_player)) {
                        log::info!("powerups: test - {} starts on the player's car", NAMES[kind as usize]);
                        self.end_effect(hi, ability);
                        self.apply(hi, kind, vehicles, ability);
                        let car = self.holders[hi].car;
                        if self.types[kind as usize].shootable {
                            self.hits.push(Hit { from: car, to: car, kind, start: self.time, key: 0x5019_ffff_0000_0000 | car as u64 });
                        }
                    }
                }
            }
        }
        self.time += dt;
        let now = self.time;
        self.hits.retain(|h| now - h.start <= 2.0);
        for s in self.sources.iter_mut().filter(|s| s.pos.is_none() && !s.dead) {
            match probe(s.spot.at, s.spot.up, s.spot.down) {
                Some(Some(p)) => s.pos = Some(p),
                Some(None) => s.dead = true,
                None => {}
            }
        }
        // the holders: the shot's clock and its search, the effect's clock
        for hi in 0..self.holders.len() {
            if let Some(mut s) = self.holders[hi].shot.take() {
                s.left -= dt;
                if s.left <= 0.0 {
                    let now = self.time;
                    for l in self.holders[hi].lasers.iter_mut() {
                        l.end = l.end.min(now);
                    }
                    // (sub_8232A4B0. Step 19g: the shot's flag +232 is set by every firing
                    // (sub_8232BB90 0x8232BC20) and cleared only here and by the clear-out, so the
                    // branch that leaves the wasted shot's icon (+164 / +176 / +192) is never taken:
                    // no icon - 19f's reading of it was wrong)
                } else {
                    let car = self.holders[hi].car;
                    if !s.found {
                        let cars = self.racer_cars(vehicles);
                        s.targets = if no_target_hook() { Vec::new() } else { targets(&cars, car, s.kind, self.holders[hi].back, &self.types[s.kind as usize]) };
                        s.found = !s.targets.is_empty();
                        s.wait = 0.0;
                    }
                    s.wait += dt;
                    if s.wait > 0.25 {
                        let (kind, list) = (s.kind, s.targets.clone());
                        self.holders[hi].shot = Some(s);
                        for to in list {
                            self.land(car, to, kind, vehicles, ability, 0);
                        }
                    } else {
                        self.holders[hi].shot = Some(s);
                    }
                }
            }
            // (the icon's second, 0x8232B34C)
            if let Some(ic) = self.holders[hi].icon.as_mut() {
                ic.2 -= dt;
                if ic.2 <= 0.0 {
                    self.holders[hi].icon = None;
                }
            }
            let mut ended = false;
            let car = self.holders[hi].car;
            // (step 19g: the Shield's timers - its update, the type's +32, runs while it is on)
            if matches!(self.holders[hi].effect.as_ref().map(|e| e.kind), Some(SHIELD | MEGA_SHIELD)) {
                self.holders[hi].fx.shield_tick(dt);
            }
            // (step 19g: the lightning's frame - the types' +32, sub_82375EB8)
            if let (Some(l), Some(c)) = (self.holders[hi].bolts.as_mut(), vehicles.cars.get(car)) {
                let bz = crate::pu_fx::bound_max_z(c);
                let (rot, pos) = (c.state.rot, c.state.pos);
                match c.model.params.hull.as_ref() {
                    Some(h) => {
                        let hit = |a: Vec3, b: Vec3| crate::pu_bolt::ray_hull(rot, pos, &h.verts, &h.tris, a, b);
                        l.update(rot, pos, bz, &hit, dt);
                    }
                    None => l.update(rot, pos, bz, &|_, _| None, dt),
                }
            }
            if let Some(e) = self.holders[hi].effect.as_mut() {
                e.left -= dt;
                if e.left <= 0.0 {
                    ended = true;
                } else if (e.kind == ICE || e.kind == 12) && !e.melted && e.left / self.types[e.kind as usize].active_time.max(1e-3) <= 0.5 {
                    // (sub_8237CFD8: the notification +96 once the ice is half gone)
                    e.melted = true;
                    let kind = e.kind;
                    self.notes.push(Note { car, kind, snd: Snd::Melt });
                }
            }
            if ended {
                self.end_effect(hi, ability);
            }
        }
        // pick-ups: the player (any source) and then every racer (not Nitro) - one each a frame
        let order: Vec<(usize, bool)> = self.holders.iter().enumerate().map(|(i, h)| (i, vehicles.cars.get(h.car).map_or(false, |c| c.is_player))).collect();
        for &(hi, player) in order.iter().filter(|o| o.1).chain(order.iter().filter(|o| !o.1)) {
            let car = self.holders[hi].car;
            let Some(c) = vehicles.cars.get(car) else { continue };
            let pos = c.state.pos;
            let prev = self.holders[hi].prev.replace(pos);
            let Some(prev) = prev else { continue };
            let full = c.state.nitro_shots >= c.state.nitro_max;
            let megas = self.holders[hi].megas();
            if full && megas {
                continue;
            }
            for si in 0..self.sources.len() {
                let s = &self.sources[si];
                let Some(sp) = s.pos else { continue };
                if s.state != 1 || (player && megas && s.kind != NITRO) || (!player && (megas || s.kind == NITRO)) {
                    continue;
                }
                // (sub_82683750: within 4 m across - having come in this frame - and between 1 m
                // below the source and its height above it)
                let r2 = 16.0;
                let d = |p: Vec3| (sp.x - p.x).powi(2) + (sp.z - p.z).powi(2);
                if pos.y + 1.0 < sp.y || pos.y > sp.y + s.spot.height || d(pos) >= r2 || d(prev) <= r2 {
                    continue;
                }
                let kind = s.kind;
                let shots = vehicles.cars[car].state.nitro_shots;
                if !self.holders[hi].accept(kind, shots, &self.types) {
                    continue;
                }
                let mut shots = shots;
                let got = self.holders[hi].give(kind, &mut shots, &self.types);
                vehicles.cars[car].state.nitro_shots = shots;
                if let Some(g) = got {
                    self.notes.push(Note { car, kind: g, snd: Snd::Pickup });
                    // (sub_82683828: back in 2 s; sub_82683A28: a new type)
                    let r = self.rand();
                    let (bits, ok) = (self.bits, self.mystery_ok);
                    let s = &mut self.sources[si];
                    s.state = 0;
                    s.back_at = self.time + 2.0;
                    let (k, m) = roll(bits, ok, r, s.mystery);
                    s.kind = k;
                    s.mystery = m;
                    break;
                }
            }
        }
        for s in &mut self.sources {
            if s.state == 0 && s.back_at <= self.time {
                s.state = 1;
            }
        }
        for h in &mut self.holders {
            if let Some(c) = vehicles.cars.get_mut(h.car) {
                c.pu_ctl = h.ctl;
                // (MegaStealth, sub_8237F8B0 / sub_8237F8F8: no collisions but the map's while it lasts)
                c.pu_ghost = h.effect.as_ref().map_or(false, |e| e.kind == MEGA_STEALTH);
                // (step 19g: the looks the car's own draw takes - see `car_look`)
                let (mirror, stealth) = car_look(h.effect.as_ref().map(|e| (e.kind, e.left)), &self.types, c.is_player);
                c.pu_mirror = mirror;
                c.pu_stealth = stealth;
                // (the vehicle's own test, sub_8232C368: the port's motorcycle rider-off is +592 0x10)
                let flags = if c.state.veh.as_ref().map_or(false, |v| v.wipe.is_some()) { 0x10 } else { 0 };
                h.veh_ok = vehicle_lets_fire(flags, if c.model.bike { 2 } else { 4 });
            }
        }
    }

    /// The racers that have finished (sub_8232A820's refusal): `done(car)`.
    pub fn set_finished(&mut self, done: &dyn Fn(usize) -> bool) {
        for h in &mut self.holders {
            h.finished = done(h.car);
        }
    }

    /// Placed, missed and waiting sources, and the nearest placed one to a point (diagnostics).
    pub fn census(&self, at: Vec3) -> (usize, usize, usize, f32) {
        let placed = self.sources.iter().filter(|s| s.pos.is_some()).count();
        let dead = self.sources.iter().filter(|s| s.dead).count();
        let near = self.sources.iter().filter_map(|s| s.pos).map(|p| p.distance(at)).fold(f32::MAX, f32::min);
        (placed, dead, self.sources.len() - placed - dead, near)
    }

    /// The sources to draw (state 1, a type under 24): position, the look's type (22 for a
    /// Mystery one).
    pub fn visible(&self) -> impl Iterator<Item = (Vec3, u8)> + '_ {
        self.sources.iter().filter(|s| s.state == 1 && s.kind < 24).filter_map(|s| s.pos.map(|p| (p, if s.mystery { MYSTERY } else { s.kind })))
    }

    /// Step 19f: the original draws no glow sprite for a source (sub_82682590 draws the icon
    /// card and the ground glow - `draw_list`, pu_draw.rs); the lamp-glow list stays empty.
    pub fn glows(&self) -> Vec<[f32; 8]> {
        Vec::new()
    }

    /// What the original draws for the power-ups this frame (step 19f, pu_draw.rs):
    /// - every source that is there (sub_82683888 -> sub_82682590 with f1 = 0 and the ground
    ///   glow on): its look (22 for a Mystery one) at its place;
    /// - (step 19g) NOT a wasted shot's icon: the holder's +164 is never set (see `update`).
    pub fn draw_list(&self) -> Vec<(Vec3, u8, f32, bool)> {
        self.visible().map(|(p, k)| (p, k, 0.0, true)).collect()
    }

    /// Everything off at the end (the Racing state's `RemovePowerUps`, sub_8232B030).
    pub fn clear(&mut self, vehicles: &mut crate::vehicles::Vehicles, ability: &mut crate::ability::Abilities) {
        for hi in 0..self.holders.len() {
            self.end_effect(hi, ability);
            self.holders[hi].shot = None;
            self.holders[hi].slots = [NONE; 2];
            self.holders[hi].ctl = 0;
            if let Some(c) = vehicles.cars.get_mut(self.holders[hi].car) {
                c.pu_ctl = 0;
                c.pu_ghost = false;
                c.pu_mirror = 0.0;
                c.pu_stealth = false;
            }
            self.holders[hi].icon = None;
            self.holders[hi].lasers.clear();
        }
        self.hits.clear();
        self.notes.clear();
    }
}

/// Test hook MCLA_POWERUP_TEST="<type>[,<frame>[,<fire frame>[,back]]]": the player is given
/// that power-up (a type number or name) at the frame (default 300) and fires it (IN_ZONE) at
/// the fire frame - backwards with a fourth field 1.
fn no_target_hook() -> bool {
    // (step 19g test hook MCLA_PU_NOTARGET=1: shots find nobody - to look at the laser for its
    // whole ShootTime)
    std::env::var("MCLA_PU_NOTARGET").is_ok()
}

pub fn test_hook() -> Option<(u8, u64, Option<u64>, bool)> {
    let v = std::env::var("MCLA_POWERUP_TEST").ok()?;
    let mut it = v.split(',');
    let k = it.next()?.trim();
    let kind = k.parse::<u8>().ok().or_else(|| NAMES.iter().position(|n| n.eq_ignore_ascii_case(k)).map(|i| i as u8))?;
    let frame = it.next().and_then(|f| f.trim().parse().ok()).unwrap_or(300);
    let fire = it.next().and_then(|f| f.trim().parse().ok());
    let back = it.next().map_or(false, |f| f.trim() == "1");
    Some((kind, frame, fire, back))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn types() -> [TypeTune; 24] {
        let mut t: [TypeTune; 24] = std::array::from_fn(|i| TypeTune::constructed(i as u8));
        t[ICE as usize].active_time = 5.0;
        t
    }

    #[test]
    fn hand_placed_rings_and_rows() {
        // ring: 3 / 4 / 5 by the width, the first one at +x
        assert_eq!(hand_spots(Vec3::ZERO, 9.0, Vec3::ZERO).len(), 3);
        assert_eq!(hand_spots(Vec3::ZERO, 10.0, Vec3::ZERO).len(), 4);
        let r = hand_spots(Vec3::new(1.0, 2.0, 3.0), 15.0, Vec3::ZERO);
        assert_eq!(r.len(), 5);
        assert!((r[0].at - Vec3::new(16.0, 2.0, 3.0)).length() < 1e-4);
        assert_eq!((r[0].up, r[0].down, r[0].height), (2.0, 20.0, 5.0));
        // row across the way (+x way -> spread along z), 9 m: three, 9 m apart
        let row = hand_spots(Vec3::ZERO, 9.0, Vec3::new(100.0, 0.0, 0.0));
        assert_eq!(row.len(), 3);
        assert!((row[0].at - Vec3::new(0.0, 0.0, 9.0)).length() < 1e-4 && (row[2].at - Vec3::new(0.0, 0.0, -9.0)).length() < 1e-4);
        let counts: Vec<usize> = [2.0, 3.0, 6.5, 10.0, 13.0].iter().map(|w| hand_spots(Vec3::ZERO, *w, Vec3::X).len()).collect();
        assert_eq!(counts, vec![1, 2, 3, 4, 5]);
        assert_eq!(hand_spots(Vec3::ZERO, 2.0, Vec3::X)[0].at, Vec3::ZERO);
    }

    #[test]
    fn layout_from_checkpoints() {
        let cp = |x: f32, r: f32| CpIn { pos: Vec3::new(x, 0.0, 0.0), line_end: None, radius: r, height: 100.0 };
        // radius 15 -> int(9/3)=3 -> even: 2, across the way (+x -> along z), 3 m either side
        let s = checkpoint_spots(&[cp(0.0, 15.0), cp(100.0, 21.0), cp(200.0, 15.0)], true);
        assert_eq!(s.iter().filter(|s| s.at.x == 0.0).count(), 2);
        assert!((s[0].at - Vec3::new(0.0, 0.0, -3.0)).length() < 1e-4);
        // radius 21 -> 4 (+-3, +-9)
        assert_eq!(s.iter().filter(|s| s.at.x == 100.0 && !s.last).count(), 4);
        // the last checkpoint takes the one before it, without a direction: stacked
        let last: Vec<&Spot> = s.iter().filter(|s| s.last).collect();
        assert_eq!(last.len(), 4);
        assert!(last.iter().all(|s| s.at == Vec3::new(100.0, 0.0, 0.0)));
        // radius 9: one, then none when ordered
        assert_eq!(checkpoint_spots(&[cp(0.0, 9.0), cp(50.0, 9.0)], true).len(), 0);
        assert_eq!(checkpoint_spots(&[cp(0.0, 9.0), cp(50.0, 9.0)], false).len(), 2);
        // unordered: the cluster in the world's axes, 6 m a unit
        let u = checkpoint_spots(&[cp(0.0, 12.0), cp(50.0, 9.0)], false);
        assert!((u[0].at - Vec3::new(3.0, 0.0, -3.0)).length() < 1e-4);
        // (the original's third: 9 x 0.33333334 is 3)
        assert_eq!(checkpoint_spots(&[cp(0.0, 15.0), cp(50.0, 9.0)], false).len(), 6);
        // a line checkpoint: along the line, (length - 5) / 3
        let line = CpIn { pos: Vec3::ZERO, line_end: Some(Vec3::new(0.0, 0.0, 20.0)), radius: 5.0, height: 50.0 };
        let l = checkpoint_spots(&[line, cp(100.0, 9.0)], true);
        assert_eq!(l.iter().filter(|s| !s.last).count(), 4);
        assert!(l.iter().all(|s| s.at.x == 0.0));
        assert_eq!((l[0].up, l[0].down, l[0].height), (1.0, 1.0, 50.0));
    }

    #[test]
    fn types_by_the_record() {
        let mut rec = [0u8; 28];
        rec[0] = 0xff;
        rec[1] = 0xe0;
        let (bits, m) = allowed(&rec);
        assert_eq!((bits, m), (0x3ff, true));
        // one in four a Mystery one; four running numbers share a type
        assert_eq!(roll(bits, true, 0, false), (NITRO, false));
        assert_eq!(roll(bits, true, 4, false), (ICE, true));
        assert_eq!(roll(bits, true, 5, true), (ICE, false));
        assert_eq!(roll(bits, true, 7, false).0, ICE);
        assert_eq!(roll(bits, true, 4 * 10, false).0, NITRO);
        // only Shield and Agro
        rec[0] = 0x04;
        rec[1] = 0x40;
        let (bits, m) = allowed(&rec);
        assert!(!m);
        assert_eq!((roll(bits, m, 0, false).0, roll(bits, m, 4, false).0, roll(bits, m, 8, false).0), (SHIELD, AGRO, SHIELD));
        assert_eq!(roll(0, false, 3, false).0, NONE);
    }

    #[test]
    fn holding_megas_and_nitro() {
        let t = types();
        let mut h = Holder::new(0);
        let mut shots = 4u8;
        assert!(h.accept(NITRO, shots, &t));
        assert_eq!(h.give(NITRO, &mut shots, &t), Some(NITRO));
        assert_eq!(shots, 5);
        assert!(!h.accept(NITRO, shots, &t));
        assert_eq!(h.give(ICE, &mut shots, &t), Some(ICE));
        // the same again with the second slot empty: MegaIce in both
        assert_eq!(h.give(ICE, &mut shots, &t), Some(12));
        assert_eq!(h.slots, [12, 12]);
        assert!(h.megas());
        assert!(!h.accept(ICE, 0, &t) && !h.accept(STOP, 0, &t));
        let mut h = Holder::new(0);
        h.give(STOP, &mut shots, &t);
        h.give(SHIELD, &mut shots, &t);
        assert_eq!(h.slots, [STOP, SHIELD]);
        assert!(h.accept(STOP, 0, &t));
        assert_eq!(h.give(STOP, &mut shots, &t), None);
        assert!(h.can_fire());
        h.shift();
        assert_eq!(h.slots, [SHIELD, NONE]);
        h.slots = [17, 17];
        h.shift();
        assert_eq!(h.slots, [NONE, NONE]);
    }

    #[test]
    fn shots_find_their_targets() {
        let t = TypeTune::constructed(ICE);
        let q = Quat::IDENTITY;
        // ahead is -z: 12 m ahead, 30 m ahead (out of range), 5 m behind, 15 m to the side
        let cars = vec![(0, Vec3::ZERO, q, false), (1, Vec3::new(0.0, 0.0, -12.0), q, false), (2, Vec3::new(0.0, 0.0, -30.0), q, false), (3, Vec3::new(0.0, 0.0, 5.0), q, false), (4, Vec3::new(15.0, 0.0, -3.0), q, false)];
        assert_eq!(targets(&cars, 0, ICE, false, &t), vec![1]);
        assert_eq!(targets(&cars, 0, ICE, true, &t), vec![3]);
        let mut hidden = cars.clone();
        hidden[1].3 = true;
        assert_eq!(targets(&hidden, 0, ICE, false, &t), Vec::<usize>::new());
        // a mega: everyone within the range, any direction
        let m = TypeTune::constructed(12);
        assert_eq!(m.width, -1.0);
        assert_eq!(targets(&cars, 0, 12, false, &m), vec![1, 3, 4]);
    }

    #[test]
    fn hud_and_controls() {
        let t = types();
        let mut h = Holder::new(0);
        assert_eq!(hud_values(&h, &t), [0.0, 0.0, 1.0]);
        h.slots = [STOP, 16];
        assert_eq!(hud_values(&h, &t), [7.0, 6.0, 1.0]);
        // a shot out: its type less 11 (as the original does it), the first slot, time / ActiveTime
        h.shot = Some(Shot { kind: ICE, left: 2.5, found: false, targets: vec![], wait: 0.0 });
        assert_eq!(hud_values(&h, &t), [-9.0, 7.0, 0.5]);
        h.shot = Some(Shot { kind: 12, left: 5.0, found: false, targets: vec![], wait: 0.0 });
        assert_eq!(hud_values(&h, &t)[0], 2.0);
        let mut i = crate::physics::car::CarInput { throttle: 1.0, steer: 0.4, ..Default::default() };
        apply_ctl(&mut i, 6);
        assert_eq!((i.throttle, i.brake), (0.0, 1.0));
        let mut i = crate::physics::car::CarInput { throttle: 1.0, steer: 0.4, ..Default::default() };
        apply_ctl(&mut i, 0x40);
        assert_eq!(i.steer, -0.4);
        apply_ctl(&mut i, 0x80);
        assert!(i.ice);
    }

    /// Step 19f: Pulse's push (sub_82380418) and the other effect data.
    #[test]
    fn pulse_push_and_effect_data() {
        let mut t = TypeTune::constructed(PULSE);
        t.force = Vec3::new(150.0, 300.0, 0.0);
        t.torque = Vec3::new(200.0, 300.0, 0.0);
        t.force_dir = Vec3::new(150.0, 300.0, 0.0);
        // a car facing -z (its right +x): A . ForceDir > 0, -(C . ForceDir) = 0 -> no torque
        let (f, q) = pulse_push(Quat::IDENTITY, &t);
        assert_eq!(f, Vec3::new(150.0, 300.0, 0.0));
        assert_eq!(q, Vec3::ZERO);
        // turned half round: its right is -x - the push's sideways part goes to world +x again
        let (f, _) = pulse_push(Quat::from_rotation_y(std::f32::consts::PI), &t);
        assert!((f - Vec3::new(150.0, 300.0, 0.0)).length() < 1e-3);
        // a quarter round (right = -z): A . ForceDir = 0 -> no sideways push; C = +x . FD > 0
        let (f, q) = pulse_push(Quat::from_rotation_y(std::f32::consts::FRAC_PI_2), &t);
        assert!((f - Vec3::new(0.0, 300.0, 0.0)).length() < 1e-3 && q.length() < 1e-3);
        // over one tick of 1/60 s: 2.5 m/s sideways, 5 m/s up
        assert!((f * TICK).y - 5.0 < 1e-4);
        // the vehicle's own test, sub_8232C368
        assert!(vehicle_lets_fire(0, 4) && !vehicle_lets_fire(0x10, 4) && vehicle_lets_fire(0x10, 2) && !vehicle_lets_fire(0x11, 2));
        // a racer that has finished takes nothing (sub_8232A820)
        let ty = types();
        let mut h = Holder::new(0);
        assert!(h.accept(ICE, 0, &ty));
        h.finished = true;
        assert!(!h.accept(ICE, 0, &ty) && !h.accept(NITRO, 0, &ty));
        h.finished = false;
        h.slots = [STOP, NONE];
        assert!(h.can_fire());
        h.veh_ok = false;
        assert!(!h.can_fire());
        // the session copy's types: all ten and RANDOM
        let mut rec = [0u8; 28];
        rec[0] = SESSION_TYPES[0];
        rec[1] = SESSION_TYPES[1];
        assert_eq!(allowed(&rec), (0x3ff, true));
        // the ground glow's fade: full to 125 m, none from 250 m
        assert_eq!((crate::pu_draw::fade(100.0), crate::pu_draw::fade(260.0)), (1.0, 0.0));
        assert!((crate::pu_draw::fade(187.5) - 0.5).abs() < 1e-5);
    }

    #[test]
    fn tune_text() {
        let v = parse_tune("type: a\nmcPowerUpMgr {\n  m_XSize 0.75 \n  Ice {\n    ActiveTime 5 \n    m_TexColor 0.08\t0.128\t0.146\t1.78 \n  }\n}\n");
        assert_eq!(v.get("m_XSize"), Some(&vec![0.75]));
        assert_eq!(v.get("Ice.ActiveTime"), Some(&vec![5.0]));
        assert_eq!(v.get("Ice.m_TexColor").map(|x| x.len()), Some(4));
    }

    /// The disc's data, where it is there: the tune file and a race script's list.
    #[test]
    fn disc_data() {
        let dir = "D:/MCLA/extracted/cache";
        if !std::path::Path::new(dir).join("tune/powerup/0xe49d3ba2").is_file() {
            return;
        }
        let t = load_types(dir);
        assert_eq!((t[ICE as usize].active_time, t[STOP as usize].active_time, t[NITRO as usize].active_time), (5.0, 3.0, 10.0));
        assert!(t[ICE as usize].shootable && t[PULSE as usize].shootable && !t[SHIELD as usize].shootable && !t[AGRO as usize].shootable);
        assert_eq!((t[PULSE as usize].force, t[PULSE as usize].force_dir), (Vec3::new(150.0, 300.0, 0.0), Vec3::new(150.0, 300.0, 0.0)));
        assert_eq!(t[18].force_dir, Vec3::new(150.0, 260.0, 0.0));
        let looks = load_looks(dir);
        assert_eq!(looks[ICE as usize].glow_color, [0.0, 0.806801, 1.0, 1.0]);
        // (step 19f: what the draw reads - every type's file has these)
        for k in [NITRO, ICE, REVERSE, STEALTH, DISRUPTER, SHIELD, STOP, PULSE, MIRROR, AGRO, MYSTERY] {
            let l = looks[k as usize];
            assert!(l.ground_glow_scale > 1.0 && l.powerup_width >= 1.0 && l.icon_height >= 2.5 && l.powerup_color[3] > 0.0, "{k} {l:?}");
        }
        let p = std::path::Path::new(dir).join("script_obj/game/race/downtown/circuit/dt_circuit_mb_01.sco");
        if let Some(sco) = std::fs::read(&p).ok().and_then(|b| mcla_formats::sco::decode(&b).ok()) {
            let s = script_sources(&sco).expect("the race's list");
            assert_eq!(s.len(), 20);
            assert!((s[0].0 - Vec3::new(1965.34, 6.84917, -810.453)).length() < 0.01);
            assert_eq!((s[0].1, s[1].1, s[19].1), (9.0, 10.5, 9.5));
            assert!((s[19].2 - Vec3::new(1461.57, -3.21943, 10.1814)).length() < 0.01);
        }
        // every race script with a list reads (CreatePowerUpSources' count of entries)
        let (mut with, mut read) = (0, 0);
        let mut bad = Vec::new();
        for d in ["beaches", "downtown", "hills", "hollywood", "southcentral", "all", "global"] {
            let Ok(kinds) = std::fs::read_dir(std::path::Path::new(dir).join("script_obj/game/race").join(d)) else { continue };
            for k in kinds.flatten() {
                let Ok(files) = std::fs::read_dir(k.path()) else { continue };
                for f in files.flatten() {
                    let Some(sco) = std::fs::read(f.path()).ok().and_then(|b| mcla_formats::sco::decode(&b).ok()) else { continue };
                    let Some(n) = sco.calls().into_iter().find(|c| c.hash == 0x8a115016).and_then(|c| c.args.first().and_then(|a| a.num())) else { continue };
                    with += 1;
                    match script_sources(&sco) {
                        Some(s) if s.len() == n as usize => read += 1,
                        other => bad.push((f.path().display().to_string(), n, other.map(|s| s.len()))),
                    }
                }
            }
        }
        assert!(with > 200 && bad.is_empty(), "{with} scripts with a list, {read} read; not read: {:?}", &bad[..bad.len().min(8)]);
    }
    /// The power-up type constructors run by the original's own code offline (tools/powerup_ctor_reference.py:
    /// the unchanged constructors and their direct callees on a flat guest memory, the two name callees
    /// - atString init and the string-table lookup, which only fill the name fields - stubbed). Per type:
    /// id, mega partner (+8), ActiveTime, Shootable, ShootTime, ShootRange, width (+32), DrawIcon, as f32 bits.
    /// Evidence: docs/reference/powerups/verify_small_evidence.md.
    #[test]
    fn constructors_match_the_original() {
        const ORIGINAL: [(u8, u8, u32, u8, u32, u32, u32, u8); 13] = [
        (25, 25, 0x41200000, 0, 0x40a00000, 0x41a00000, 0x41200000, 1), // F-34625 base
        (0, 25, 0x41200000, 0, 0x40a00000, 0x41a00000, 0x41200000, 1), // F-86562
        (2, 13, 0x41200000, 1, 0x40a00000, 0x41a00000, 0x41200000, 1), // F-47630
        (13, 13, 0x41200000, 1, 0x40a00000, 0x41a00000, 0xbf800000, 1), // F-88704
        (3, 14, 0x41200000, 0, 0x40a00000, 0x41a00000, 0x41200000, 1), // F-89556
        (14, 14, 0x41200000, 0, 0x40a00000, 0x41a00000, 0x41200000, 1), // F-17229
        (8, 19, 0x41200000, 0, 0x40a00000, 0x41a00000, 0x41200000, 1), // F-43339
        (19, 19, 0x41200000, 0, 0x40a00000, 0x41a00000, 0x41200000, 1), // F-41737
        (9, 20, 0x41200000, 0, 0x40a00000, 0x41a00000, 0x41200000, 1), // F-59527
        (20, 20, 0x41200000, 0, 0x40a00000, 0x41a00000, 0x41200000, 1), // F-56106
        (10, 25, 0x41200000, 0, 0x40a00000, 0x41a00000, 0x41200000, 1), // F-77222
        (21, 25, 0x41200000, 0, 0x40a00000, 0x41a00000, 0x41200000, 1), // F-98423
        (18, 18, 0x41200000, 1, 0x40a00000, 0x41a00000, 0xbf800000, 1), // F-44279
        ];
        for (id, mega, active, shoot, st, sr, w, icon) in ORIGINAL {
            let t = TypeTune::constructed(id);
            assert_eq!(t.mega, mega, "type {id} mega partner");
            assert_eq!(t.active_time.to_bits(), active, "type {id} ActiveTime");
            assert_eq!(t.shootable, shoot != 0, "type {id} Shootable");
            assert_eq!(t.shoot_time.to_bits(), st, "type {id} ShootTime");
            assert_eq!(t.shoot_range.to_bits(), sr, "type {id} ShootRange");
            assert_eq!(t.width.to_bits(), w, "type {id} width");
            assert_eq!(t.draw_icon, icon != 0, "type {id} DrawIcon");
            assert_eq!(t.fade.map(f32::to_bits), [0x3f333333, 0x41a00000, 0x40800000, 0x40000000], "type {id} fade");
        }
    }

    /// The original's options-record reader and the k-th-allowed-type helper, the two Stealth getters, the holder
    /// constructor and the look constructor, run offline (tools/powerups_small_reference.py): table
    /// tests/powerups_small_reference.txt. Evidence: docs/reference/powerups/verify_small_evidence.md section 2.
    #[test]
    fn small_functions_match_the_original() {
        const REF: &str = include_str!("../tests/powerups_small_reference.txt");
        let hex = |s: &str| u32::from_str_radix(s, 16).unwrap();
        let types: [TypeTune; 24] = std::array::from_fn(|i| TypeTune::constructed(i as u8));
        let dir = std::env::temp_dir().join("mcla_powerup_look_defaults");
        std::fs::create_dir_all(dir.join("tune/powerup")).unwrap();
        for t in 0..24 {
            let file = format!("0x{:08x}", mcla_formats::hash::joaat(&format!("powerup_{}.mcpowerup", NAMES[t])));
            std::fs::write(dir.join("tune/powerup").join(file), "type: mcPowerup
").unwrap();
        }
        let looks = load_looks(dir.to_str().unwrap());
        let mut n = [0usize; 4];
        for line in REF.lines() {
            let w: Vec<&str> = line.split_whitespace().collect();
            match w[0] {
                "OPT" => {
                    let mut rec = [0u8; 28];
                    rec[0] = hex(w[1]) as u8;
                    rec[1] = hex(w[2]) as u8;
                    let (bits, mystery) = allowed(&rec);
                    assert_eq!(bits, hex(w[4]), "allowed word of {line}");
                    assert_eq!(mystery, w[3] != "00", "RANDOM flag of {line}");
                    for (k, s) in w[5..].iter().enumerate() {
                        let want: i32 = s.parse().unwrap();
                        if k < bits.count_ones() as usize {
                            assert_eq!(roll(bits, false, (k as i32) << 2, false).0 as i32, want, "k={k} of {line}");
                        } else {
                            assert_eq!(want, -1, "past the end: {line}");
                        }
                    }
                    n[0] += 1;
                }
                "HLD" => {
                    let k: i32 = w[1].parse().unwrap();
                    let want = w[2] != "0" || w[3] != "0";
                    let effect = if k < 0 { None } else { Some((k as u8, types.get(k as usize).map_or(1.0, |t| t.active_time))) };
                    assert_eq!(car_look(effect, &types, true).1, want, "stealth getters, effect type {k}");
                    n[1] += 1;
                }
                "NEW" => {
                    let b: Vec<u8> = w[1..].iter().map(|x| hex(x) as u8).collect();
                    let u = |o: usize| u32::from_be_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
                    let h = Holder::new(0);
                    assert_eq!((u(136), u(140)), (h.slots[0] as u32, h.slots[1] as u32));
                    assert_eq!((u(228) == 0, u(236) == 0, u(104)), (h.shot.is_none(), h.effect.is_none(), h.shield_hits as u32));
                    n[2] += 1;
                }
                "LOOK" => {
                    let b: Vec<u8> = w[1..].iter().map(|x| hex(x) as u8).collect();
                    let f = |o: usize| u32::from_be_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
                    let c = |o: usize| [f(o), f(o + 4), f(o + 8), f(o + 12)];
                    let bits = |a: [f32; 4]| a.map(f32::to_bits);
                    for l in &looks {
                        assert_eq!(l.sphere_scale.to_bits(), f(12));
                        assert_eq!(l.ground_glow_scale.to_bits(), f(28));
                        assert_eq!(l.powerup_width.to_bits(), f(32));
                        assert_eq!(l.icon_height.to_bits(), f(48));
                        assert_eq!(l.powerup_height.to_bits(), f(52));
                        assert_eq!(bits(l.glow_color), c(64));
                        assert_eq!(bits(l.sphere_color), c(80));
                        assert_eq!(bits(l.powerup_color), c(112));
                        assert_eq!(bits(l.ground_glow_color1), c(128));
                        assert_eq!(bits(l.ground_glow_color2), c(144));
                        assert_eq!(bits(l.beam_color), c(192));
                    }
                    n[3] += 1;
                }
                _ => {}
            }
        }
        assert_eq!(n, [2048, 32, 1, 1]);
    }
}
