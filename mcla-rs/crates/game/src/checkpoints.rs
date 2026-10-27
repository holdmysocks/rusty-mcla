//! Race checkpoints as the original builds, tests and shows them.
//!
//! Sources (static recompilation, image base 0x82000000; notes in `SAVE_WORK.md`, "Race
//! checkpoints"):
//! - the checkpoint object (368 bytes, constructor `F-64962`): +0 the look ("type") per
//!   racer (16 words), +64 position, +80 second position (a line), +96 the gate's offset,
//!   +112 where it is drawn, +164 5, +168 activation radius (10), +172 where along a line it is
//!   drawn (0.5), +176 height (100), +180 activation height (50), +184 3 (the depth below),
//!   +188 max speed (-1), +200 state per racer (3 closed; 2 opening, 0 open, 1 closing),
//!   +264 their timers, +330 a line, +336 / +340 the two effects, +348 its light, +352 the
//!   light's flicker phase. Natives `F-63293` (`Checkpoint_*`), `F-38379`
//!   (`CheckpointManager_*`).
//! - the hit test `F-78734`, asked by `CheckpointManager_HasHitCheckpoint`
//!   (`F-51496`) and `..._GetWhichHitCheckpoint` (`F-37765`) with the car's last
//!   position and a point 2 m ahead of it along its velocity.
//! - the race scripts set every checkpoint from their tables (positions, radii, heights,
//!   activation heights, arrow look-at x / z, second positions, draw fractions, "Normal" /
//!   "Line", max speeds - e.g. `be_circuit_jj_02.sco` 0x774b..0x78bc);
//!   OrderedCheckpointCore (`script_obj/game/0x3ae26fc8/0xf637b18a` 0x824) and the unordered
//!   core (`0xd82fca9b` 0x745, 0x16e0) choose the looks.
//! - the looks: `tune/checkpoint/<name>.mccp` (`mcCP`, members `F-15093`, defaults
//!   `F-26586`, loader and its fixed disc colours `F-16373`); the per-frame update
//!   `F-76770` plays the effect `checkpoint_ctf_generic` (the road flare) tinted with
//!   `m_ParticleColor` and, for some looks, `light_column` tinted with `m_LightColumnColor`.

use glam::Vec3;
use std::path::Path;

/// The looks in the order of the executable's name table (0x827DD028).
pub const NAMES: [&str; 18] = [
    "cp", "activecp", "finalcp", "finallapcp", "unorderedcp", "ctfpickupnoteam", "ctfpickupblueteam", "ctfpickupredteam", "ctfdropoffnoteam", "ctfdropoffblueteam", "ctfdropoffredteam", "redcp", "bluecp", "greencp", "sideracecp",
    "tournamentcp", "fillercp", "lightcolumncp",
];
pub const CP: u8 = 0;
pub const FINAL: u8 = 2;
pub const FINAL_LAP: u8 = 3;
pub const UNORDERED: u8 = 4;
pub const CTF_PICKUP_NOTEAM: u8 = 5;
pub const CTF_DROPOFF_NOTEAM: u8 = 8;
pub const LIGHT_COLUMN: u8 = 17;

/// One `mcCP` record (offsets of the record in brackets).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CpTune {
    /// (+16 / +32) the checkpoint light's two colours, (+48) the flare's tint, (+64) the
    /// column's tint, (+80) `m_ParticleShadowColor`, (+96) `m_DaylightGlowSizeAlpha`
    pub light1: [f32; 3],
    pub light2: [f32; 3],
    pub particle: [f32; 3],
    pub column: [f32; 3],
    pub shadow: [f32; 3],
    pub glow: [f32; 3],
    /// (+112 .. +160) the editor's discs
    pub inner1: [f32; 4],
    pub inner2: [f32; 4],
    pub outer1: [f32; 4],
    pub outer2: [f32; 4],
    /// (+176, +180, +192) the icon
    pub icon_scale: f32,
    pub icon_height: f32,
    pub icon_color: [f32; 4],
    /// (+208, +212) the light's flicker
    pub flicker_random: f32,
    pub flicker_speed: f32,
    /// (+216, +220, +224)
    pub color_influence: f32,
    pub opacity_influence: f32,
    pub opacity: f32,
}

impl Default for CpTune {
    /// The constructor's values (`F-26586`).
    fn default() -> Self {
        let disc = [0.15, 0.95, 0.9, 0.25];
        CpTune {
            light1: [0.75, 0.75, 1.0],
            light2: [0.75, 0.75, 1.0],
            particle: [1.0; 3],
            column: [1.0; 3],
            shadow: [1.0; 3],
            glow: [5.0, 5.0, 0.25],
            inner1: disc,
            inner2: disc,
            outer1: disc,
            // (the second outer colour's alpha lane is overwritten with 0 after the copy of the disc colour)
            outer2: [disc[0], disc[1], disc[2], 0.0],
            icon_scale: 7.0,
            icon_height: 10.0,
            icon_color: [0.15, 0.95, 0.9, 1.0],
            flicker_random: 1.0,
            flicker_speed: 1.0,
            color_influence: 0.0,
            opacity_influence: 0.0,
            opacity: 0.5,
        }
    }
}

/// The tune folder (`F-19793`; the original's developer build reads `t://mc4//assets//tune//checkpoint` when its
/// developer flag is set, which the shipped game never does).
pub const TUNE_DIR: &str = "tune/checkpoint";

/// The 18 looks: each file read over the constructor's values, then the loader's fixed disc
/// colours for cp, activecp, finalcp, finallapcp and greencp (`F-16373`; they equal the
/// files' own).
pub fn load_tunes(data_dir: &str) -> Vec<CpTune> {
    let dir = Path::new(data_dir).join(TUNE_DIR);
    let mut out: Vec<CpTune> = NAMES.iter().map(|n| {
        let mut t = CpTune::default();
        let file = dir.join(format!("0x{:08x}", mcla_formats::hash::joaat(&format!("{n}.mccp"))));
        if let Ok(text) = std::fs::read_to_string(&file) {
            parse_tune(&text, &mut t);
        }
        t
    }).collect();
    let (a, b, c, d, e, f, g) = (0.95f32, 0.15f32, 0.45f32, 0.65f32, 0.85f32, 0.55f32, 0.25f32);
    let set = |t: &mut CpTune, i: [f32; 4], o1: [f32; 4], o2: [f32; 4]| {
        t.inner1 = i;
        t.inner2 = i;
        t.outer1 = o1;
        t.outer2 = o2;
    };
    set(&mut out[3], [a, a, a, f], [a, a, a, g], [a, a, a, g]);
    set(&mut out[13], [b, a, b, c], [b, a, b, b], [b, a, b, b]);
    set(&mut out[2], [a, b, b, c], [a, b, b, b], [a, b, b, b]);
    set(&mut out[0], [a, d, b, c], [e, f, b, b], [e, f, b, b]);
    set(&mut out[1], [a, a, a, f], [a, b, b, c], [a, b, b, c]);
    out
}

fn parse_tune(text: &str, t: &mut CpTune) {
    for line in text.lines() {
        let mut it = line.split_whitespace();
        let Some(key) = it.next() else { continue };
        let v: Vec<f32> = it.filter_map(|x| x.parse().ok()).collect();
        let v3 = |d: &mut [f32; 3]| {
            if v.len() >= 3 {
                *d = [v[0], v[1], v[2]];
            }
        };
        let v4 = |d: &mut [f32; 4]| {
            if v.len() >= 4 {
                *d = [v[0], v[1], v[2], v[3]];
            }
        };
        let f = |d: &mut f32| {
            if let Some(x) = v.first() {
                *d = *x;
            }
        };
        match key {
            "m_lightColor1" => v3(&mut t.light1),
            "m_lightColor2" => v3(&mut t.light2),
            "m_ParticleColor" => v3(&mut t.particle),
            "m_ParticleShadowColor" => v3(&mut t.shadow),
            "m_LightColumnColor" => v3(&mut t.column),
            "m_DaylightGlowSizeAlpha" => v3(&mut t.glow),
            "m_innerDiscColor1" => v4(&mut t.inner1),
            "m_innerDiscColor2" => v4(&mut t.inner2),
            "m_outerDiscColor1" => v4(&mut t.outer1),
            "m_outerDiscColor2" => v4(&mut t.outer2),
            "m_iconColor" => v4(&mut t.icon_color),
            "m_iconScale" => f(&mut t.icon_scale),
            "m_iconHeight" => f(&mut t.icon_height),
            "m_flickerRandom" => f(&mut t.flicker_random),
            "m_flickerSpeed" => f(&mut t.flicker_speed),
            "m_GlobalLightColorInfluence" => f(&mut t.color_influence),
            "m_GlobalLightOpacityInfluence" => f(&mut t.opacity_influence),
            "m_Opacity" => f(&mut t.opacity),
            _ => {}
        }
    }
}

/// `Checkpoint_SetArrowLookatPosition` (`F-27552`): the vector (x, 0, z) is stored at +144, and the flag +331 is set
/// unless x and z are both 0 (comparing as floats: -0 is 0, NaN is not).
pub fn look_at_of(x: f32, z: f32) -> Option<Vec3> {
    (x != 0.0 || z != 0.0).then(|| Vec3::new(x, 0.0, z))
}

/// The looks that get the light column besides the flare: the jump table at A-90356
/// (finalcp, the six flag pick-up / drop-off looks, lightcolumncp).
pub fn has_column(kind: u8) -> bool {
    matches!(kind, 2 | 5..=10 | 17)
}

// ---------------------------------------------------------------------------------
// The checkpoint and its hit test

/// One checkpoint as the race's script sets it up (the object's defaults where it does not).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CpSpec {
    /// +64
    pub pos: Vec3,
    /// +80 with +330: a line from `pos` to here
    pub second: Option<Vec3>,
    /// +172: where along the line the marker stands (0.5)
    pub draw_line: f32,
    /// +168 (10)
    pub radius: f32,
    /// +176 (100): the flare effect's "Height" and the power-up layout's height
    pub height: f32,
    /// +180 (50): how far above the checkpoint it still counts
    pub act_height: f32,
    /// +188 (-1): `Checkpoint_SetMaxSpeed` x 0.447 when above 0
    pub max_speed: f32,
    /// +144 with +331: `Checkpoint_SetArrowLookAt` (`F-27552`): (x, 0, z), set when x or z
    /// is not 0 - where the arrows projected onto the smoke point; none: the next checkpoint
    pub look_at: Option<Vec3>,
}

/// +184: how far below the checkpoint it still counts (no native sets it).
pub const DEPTH: f32 = 3.0;
/// +96: the line is also crossed 10 m before and after it (`F-34461`).
pub const GATE_BAND: f32 = 10.0;

impl CpSpec {
    pub fn at(pos: Vec3) -> CpSpec {
        CpSpec { pos, second: None, draw_line: 0.5, radius: 10.0, height: 100.0, act_height: 50.0, max_speed: -1.0, look_at: None }
    }

    pub fn with_radius(pos: Vec3, radius: f32) -> CpSpec {
        CpSpec { radius, ..CpSpec::at(pos) }
    }

    /// +112: where the marker stands (before the original's 2 m probe of the ground).
    pub fn draw_pos(&self) -> Vec3 {
        match self.second {
            Some(b) => self.pos + (b - self.pos) * self.draw_line,
            None => self.pos,
        }
    }

    /// +96: the line's normal in the ground plane, 10 m long - (dz, 0, -dx) of the line.
    pub fn gate_offset(&self) -> Vec3 {
        match self.second {
            Some(b) => {
                let d = b - self.pos;
                Vec3::new(d.z, 0.0, -d.x).normalize_or_zero() * GATE_BAND
            }
            None => Vec3::ZERO,
        }
    }

    /// `F-78734`: did the way from `prev` (the car's last position) to `ahead` (2 m on
    /// along its velocity) touch the checkpoint.
    pub fn hit(&self, ahead: Vec3, prev: Vec3) -> bool {
        match self.second {
            Some(b) => {
                let a = self.pos;
                let off = self.gate_offset();
                for (ga, gb) in [(a, b), (a + off, b + off), (a - off, b - off)] {
                    let Some((t, u)) = cross_xz(ahead, prev, ga, gb) else { continue };
                    if !(0.0..=1.0).contains(&t) || !(-0.2..=1.2).contains(&u) {
                        continue;
                    }
                    // (the first line crossed decides; the heights are those of the line itself)
                    let gy = a.y + (b.y - a.y) * u;
                    let py = ahead.y + (prev.y - ahead.y) * t;
                    return py > gy - DEPTH && py < gy + self.act_height;
                }
                false
            }
            None => {
                let c = self.pos;
                let d = ahead - prev;
                let num = -d.dot(prev - c);
                let dd = d.dot(d);
                // the point of the way nearest the checkpoint; inside the way the reach is 1.2 x
                let (q, r) = if num <= 1e-12 {
                    (prev, self.radius)
                } else if dd - num <= 1e-12 {
                    (ahead, self.radius)
                } else {
                    (prev + d * (num / dd), self.radius * 1.2)
                };
                let (dx, dz) = (q.x - c.x, q.z - c.z);
                dx * dx + dz * dz < r * r && q.y > c.y - DEPTH && q.y < c.y + self.act_height
            }
        }
    }
}

/// The checkpoint of a race's list (`Race::checkpoints` holds where each is drawn, 0.2 m up)
/// among the lap's specs.
pub fn spec_at(specs: &[CpSpec], marker: Vec3) -> Option<&CpSpec> {
    // (an unordered race's computer drivers keep the file's own positions, without the 0.2)
    specs.iter().find(|s| {
        let d = s.draw_pos() - marker;
        d.x * d.x + d.z * d.z < 0.01 && d.y.abs() < 0.5
    })
}

/// `F-51496`: the point 2 m ahead of a car along its velocity (none when it stands).
pub fn ahead_point(pos: Vec3, vel: Vec3) -> Vec3 {
    pos + vel.normalize_or_zero() * 2.0
}

/// `F-66886` on the ground plane (x, z): where the line a0 -> a1 meets b0 -> b1, as the
/// fractions (t along a, u along b); None when they are parallel.
pub fn cross_xz(a0: Vec3, a1: Vec3, b0: Vec3, b1: Vec3) -> Option<(f32, f32)> {
    const E: f32 = 1e-5;
    let (dax, day) = (a1.x - a0.x, a1.z - a0.z);
    let (dbx, dby) = (b1.x - b0.x, b1.z - b0.z);
    // (the original's fused multiply-subtract / multiply-add: one rounding where the code has fmsubs / fmadds)
    let den = dby.mul_add(dax, -(dbx * day));
    if den.abs() < E {
        return None;
    }
    if dbx.abs() < E {
        if dax.abs() < E {
            return None;
        }
        let t = (b0.x - a0.x) / dax;
        return Some((t, (t.mul_add(day, a0.z) - b0.z) / dby));
    }
    if dby.abs() < E {
        if day.abs() < E {
            return None;
        }
        let t = (b0.z - a0.z) / day;
        return Some((t, (t.mul_add(dax, a0.x) - b0.x) / dbx));
    }
    let t = (a0.z - b0.z).mul_add(dbx, (b0.x - a0.x) * dby) / den;
    Some((t, (t.mul_add(dax, a0.x) - b0.x) / dbx))
}

// ---------------------------------------------------------------------------------
// The script's tables

fn find(dir: &Path, name: &str) -> Option<std::path::PathBuf> {
    let p = dir.join(name);
    if p.is_file() {
        return Some(p);
    }
    let p = dir.join(format!("0x{:08x}", mcla_formats::hash::joaat(name)));
    p.is_file().then_some(p)
}

#[derive(Debug)]
enum Arr {
    V(Vec<Vec3>),
    N(Vec<f32>),
    S(Vec<String>),
}

/// The run of tables from token `i` (each "base, count", then the entries: vectors as x, y,
/// z, 3, k, 3, base; single values as v, k, 1, base).
fn tables(t: &[mcla_formats::sco::Tok], mut i: usize) -> Vec<Arr> {
    use mcla_formats::sco::Tok;
    let num = |i: usize| if let Some(Tok::Num(v)) = t.get(i) { Some(*v as f32) } else { None };
    let mut out = Vec::new();
    loop {
        let (Some(base), Some(n)) = (num(i), num(i + 1)) else { break };
        let n = n as usize;
        if !(1..=400).contains(&n) {
            break;
        }
        let is_vec = |k: usize| {
            let o = i + 2 + k * 7;
            num(o).is_some() && num(o + 1).is_some() && num(o + 2).is_some() && num(o + 3) == Some(3.0) && num(o + 4) == Some(k as f32) && num(o + 5) == Some(3.0) && num(o + 6) == Some(base)
        };
        let is_one = |k: usize| {
            let o = i + 2 + k * 4;
            t.get(o).is_some() && num(o + 1) == Some(k as f32) && num(o + 2) == Some(1.0) && num(o + 3) == Some(base)
        };
        if (0..n).all(is_vec) {
            out.push(Arr::V((0..n).map(|k| Vec3::new(num(i + 2 + k * 7).unwrap(), num(i + 3 + k * 7).unwrap(), num(i + 4 + k * 7).unwrap())).collect()));
            i += 2 + n * 7;
        } else if (0..n).all(is_one) {
            if matches!(t.get(i + 2), Some(Tok::Str(_))) {
                out.push(Arr::S((0..n).map(|k| if let Some(Tok::Str(s)) = t.get(i + 2 + k * 4) { s.clone() } else { String::new() }).collect()));
            } else {
                out.push(Arr::N((0..n).map(|k| num(i + 2 + k * 4).unwrap_or(0.0)).collect()));
            }
            i += 2 + n * 4;
        } else {
            break;
        }
    }
    out
}

/// The checkpoints a race's script sets up: its ten tables in the order the script fills
/// them - positions, radii, heights, activation heights, the arrow's look-at x and z, second
/// positions, draw fractions, "Normal" / "Line", max speeds. `first`: the race file's first
/// checkpoint, to tell the table from others that look alike (none: the table run with the
/// kinds).
pub fn script_specs(data_dir: &str, race_id: &str, first: Option<Vec3>) -> Option<Vec<CpSpec>> {
    use mcla_formats::sco::Tok;
    let rel = race_id.to_ascii_lowercase();
    let (folder, name) = rel.rsplit_once('/')?;
    let dir = Path::new(data_dir).join("script_obj").join(folder);
    let script = find(&dir, &format!("{name}.sco"))?;
    let sco = mcla_formats::sco::decode(&std::fs::read(script).ok()?).ok()?;
    let t = sco.consts();
    let num = |i: usize| if let Some(Tok::Num(v)) = t.get(i) { Some(*v as f32) } else { None };
    (0..t.len().saturating_sub(9))
        .filter(|&i| matches!((num(i + 2), num(i + 3), num(i + 4)), (Some(x), Some(y), Some(z)) if first.map_or(true, |f| Vec3::new(x, y, z).distance(f) < 6.0)) && num(i + 5) == Some(3.0) && num(i + 6) == Some(0.0) && num(i + 7) == Some(3.0) && num(i + 8) == num(i))
        .find_map(|at| specs_from(&tables(&t, at)))
}

fn specs_from(a: &[Arr]) -> Option<Vec<CpSpec>> {
    let (Some(Arr::V(pos)), Some(Arr::N(radius)), Some(Arr::N(height)), Some(Arr::N(act)), Some(Arr::N(lx)), Some(Arr::N(lz)), Some(Arr::V(second)), Some(Arr::N(frac)), Some(Arr::S(kind)), Some(Arr::N(speed))) =
        (a.first(), a.get(1), a.get(2), a.get(3), a.get(4), a.get(5), a.get(6), a.get(7), a.get(8), a.get(9))
    else {
        return None;
    };
    let n = pos.len();
    if [radius.len(), height.len(), act.len(), lx.len(), lz.len(), second.len(), frac.len(), kind.len(), speed.len()].iter().any(|l| *l != n) {
        return None;
    }
    Some(
        (0..n)
            .map(|k| CpSpec {
                pos: pos[k],
                second: kind[k].eq_ignore_ascii_case("Line").then_some(second[k]),
                draw_line: frac[k],
                radius: radius[k],
                height: height[k],
                act_height: act[k],
                max_speed: if speed[k] > 0.0 { speed[k] * 0.447 } else { -1.0 },
                look_at: look_at_of(lx[k], lz[k]),
            })
            .collect(),
    )
}

// ---------------------------------------------------------------------------------
// Which look a checkpoint wears

/// OrderedCheckpointCore 0x824: a race of one checkpoint shows lightcolumncp; the last of a
/// lap finalcp on the last lap, finallapcp before; any other cp.
pub fn ordered_kind(idx_in_lap: usize, lap_len: usize, lap: usize, laps: usize) -> u8 {
    if lap_len <= 1 {
        LIGHT_COLUMN
    } else if idx_in_lap + 1 >= lap_len {
        if lap + 1 >= laps.max(1) {
            FINAL
        } else {
            FINAL_LAP
        }
    } else {
        CP
    }
}

// ---------------------------------------------------------------------------------
// The markers in the world

/// What the player's open checkpoints show: per checkpoint the time it has been opening
/// (`F-76770`: state 2 runs its timer up at 2 a second, then it is open; the flare's
/// render setup takes that as its opacity). A checkpoint no longer open stops its effects -
/// what they let out lives on.
#[derive(Default)]
pub struct Markers {
    pub tunes: Vec<CpTune>,
    open: std::collections::HashMap<u64, f32>,
    /// the light's flicker phase per open checkpoint (+352)
    phase: std::collections::HashMap<u64, f32>,
    /// checkpoints closing (state 1): key, timer (4 a second), the light's place, the look
    closing: Vec<(u64, f32, Vec3, u8)>,
    /// where each open checkpoint stood and its look (for its closing)
    last: std::collections::HashMap<u64, (Vec3, u8)>,
    /// set by the caller before `update`: what the projected arrows show
    pub arrows: Option<ArrowPlan>,
    /// seconds since the arrows were first drawn (`F-85367` keeps the time base's first
    /// reading at 0x8286ECC8)
    clock: f32,
    rng: u32,
    /// the checkpoint light the dynamic lights carry this frame (`F-62430`): the enabled
    /// one nearest the camera within 500 m - its place (3.25 m above the marker) and colour (x 4)
    pub light: Option<(Vec3, [f32; 3])>,
}

/// The arrows projected onto the low smoke (`F-85367` draws them, `F-45072` /
/// `F-71936` hand them to the flares' render setups):
/// - `from`: +112 of the first checkpoint, in list order, that is open or opening for the player;
/// - `to`: its arrow look-at (+144, when the script set one) or the next checkpoint's +64;
/// - `on`: the keys whose render setup has EffectPos.w = 1 - every open / opening checkpoint but
///   the last of the list (unless that one wears finallapcp); none in a flag game.
#[derive(Clone, Debug, Default)]
pub struct ArrowPlan {
    pub from: Vec3,
    pub to: Vec3,
    pub on: Vec<u64>,
}

/// The arrow plan of a race: `shown` the player's open checkpoints (after the ground probe),
/// `raw` where each stood before it (to find it in the script's list).
pub fn arrow_plan(race: &crate::game::Race, shown: &[Shown], raw: &[Vec3], unordered: bool) -> Option<ArrowPlan> {
    if shown.is_empty() || raw.len() != shown.len() {
        return None;
    }
    let specs = &race.cp_specs;
    let idx: Vec<Option<usize>> = raw.iter().map(|p| {
        specs.iter().position(|s| {
            let d = s.draw_pos() - (*p + Vec3::Y * 0.2);
            d.x * d.x + d.z * d.z < 0.01 && d.y.abs() < 0.5
        })
    }).collect();
    let mut plan = ArrowPlan::default();
    if !specs.is_empty() && idx.iter().all(|i| i.is_some()) {
        let n = specs.len();
        let (first, i) = idx.iter().enumerate().map(|(k, i)| (k, i.unwrap())).min_by_key(|(_, i)| *i)?;
        plan.from = shown[first].pos;
        plan.to = specs[i].look_at.unwrap_or(specs[(i + 1) % n].pos);
        for (k, s) in shown.iter().enumerate() {
            if idx[k].unwrap() + 1 < n || s.kind == FINAL_LAP {
                plan.on.push(s.key);
            }
        }
    } else {
        // (races without the script's tables: the race's own list)
        let next = race.next.first().copied().unwrap_or(0);
        let n = race.checkpoints.len();
        if next >= n {
            return None;
        }
        plan.from = shown[0].pos;
        plan.to = race.checkpoints[(next + 1) % n];
        let ll = race.lap_len.max(1);
        for s in shown {
            let last = if unordered { s.kind == LIGHT_COLUMN } else { next % ll + 1 >= ll };
            if !last || s.kind == FINAL_LAP {
                plan.on.push(s.key);
            }
        }
    }
    Some(plan)
}

/// The turn of the arrow quad drawn into the 128 x 128 target (`F-85367`): about the
/// target camera's axis by `pi / 2 + atan2(c x d, c . d)` and then about x by
/// `(1 - |c . d|) pi + 1.9 |c . d|`, where d = the unit vector from the checkpoint to where the
/// arrow points and c = the unit vector from it to the camera (both in 3D, taken in x and z).
pub fn arrow_angles(from: Vec3, to: Vec3, cam: Vec3) -> (f32, f32) {
    let d = (to - from).normalize_or_zero();
    let c = (cam - from).normalize_or_zero();
    let dot = c.x * d.x + c.z * d.z;
    let cross = c.x * d.z - c.z * d.x;
    let z = std::f32::consts::FRAC_PI_2 + cross.atan2(dot);
    let x = (1.0 - dot.abs()) * std::f32::consts::PI + 1.9 * dot.abs();
    (z, x)
}

/// Which of the four `checkpoint_arrows_anim_*` textures is drawn: 1..3 in turn at 4 a second
/// (the arrow object's +76 = 4; `A-47848`), from the first draw on. (`anim_0`, still and
/// spun with the clock, is for a game mode the port does not have: 0x8286EBF8 +3096 == 2.)
pub fn arrow_frame(t: f32) -> u32 {
    ((t * 4.0) as i32).rem_euclid(3) as u32 + 1
}

/// The checkpoint light's colour (`F-76770` A-78366..A-84641, before the light
/// manager's x 4): opening - m_lightColor1 x the timer; open - between m_lightColor1 and
/// m_lightColor2 by (sin(phase / 8) + 1) / 2; closing - m_lightColor2 x (1 - its timer).
pub fn light_color(t: &CpTune, opening: Option<f32>, phase: f32, closing: Option<f32>) -> [f32; 3] {
    let (a, b) = (Vec3::from(t.light1), Vec3::from(t.light2));
    let c = if let Some(k) = closing {
        b * (1.0 - k)
    } else if let Some(k) = opening {
        a * k
    } else {
        a + (b - a) * (((phase * 0.125).sin() + 1.0) * 0.5)
    };
    c.into()
}

/// One step of the flicker phase: on by (random 0..1 x m_flickerRandom + m_flickerSpeed) x dt,
/// back by pi once past it.
pub fn flicker_step(t: &CpTune, phase: f32, rand01: f32, dt: f32) -> f32 {
    let p = phase + (rand01 * t.flicker_random + t.flicker_speed) * dt;
    if p > std::f32::consts::PI {
        p - std::f32::consts::PI
    } else {
        p
    }
}

/// The light manager's gain on the colour (`F-39031`, 0x827E25F4) and the light's height
/// over the marker (`F-41523` 0.25 + `F-63326` 3, 0x827E25F0); its strength +64 = 40
/// for all 64 slots (`F-66001`): colour x sat(40 / d squared) x n.l, all round.
pub const LIGHT_GAIN: f32 = 4.0;
pub const LIGHT_HEIGHT: f32 = 3.25;
pub const LIGHT_INTENSITY: f32 = 40.0;

/// One open checkpoint: a key that stays the same while it is open, where it stands, its look.
#[derive(Clone, Copy, Debug)]
pub struct Shown {
    pub key: u64,
    pub pos: Vec3,
    pub kind: u8,
    /// +176 (`Checkpoint_SetHeight`, 100 by default): the flare's "Height" evolution
    pub height: f32,
}

/// The flare's "Height" evolution value: (height - 13) / (34 - 13), 0..1 (`F-76770`
/// A-21072..A-52077; 13 and 34 put at 0x8286EF90 / 0x827DD01C by `F-16373`).
pub fn height_evo(height: f32) -> f32 {
    ((height - 13.0) / (34.0 - 13.0)).clamp(0.0, 1.0)
}

/// The light column's "time" evolution value from the clock: hours / 12 up to noon, then down
/// again - 0 at midnight, 1 at noon (`F-76770` A-28517..A-88081).
pub fn time_evo(hour: f32) -> f32 {
    let h = hour.rem_euclid(24.0);
    if h < 12.0 {
        (h / 12.0).clamp(0.0, 1.0)
    } else {
        1.0 - ((h - 12.0) / 12.0).clamp(0.0, 1.0)
    }
}

const FLARE: &str = "checkpoint_ctf_generic";
const COLUMN: &str = "light_column";

impl Markers {
    pub fn new(data_dir: &str) -> Markers {
        Markers { tunes: load_tunes(data_dir), rng: 0x2545_f491, ..Default::default() }
    }

    fn rand01(&mut self) -> f32 {
        // (the game's own generator - 0x82860880 - is shared with everything else: only its
        // distribution, 0..1 in steps of 2^-23, is kept)
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        (x & 0x7f_ffff) as f32 * 1.192_092_9e-7
    }

    pub fn update(&mut self, shown: &[Shown], dt: f32, fx: &mut crate::particles::Particles, cam: Vec3) {
        // checkpoints no longer open close: their light fades over 0.25 s (state 1)
        let gone: Vec<(u64, f32)> = self.open.iter().filter(|(k, _)| !shown.iter().any(|s| s.key == **k)).map(|(k, v)| (*k, *v)).collect();
        for (k, _) in &gone {
            if let Some(&(pos, kind)) = self.last.get(k) {
                self.closing.push((*k, 0.0, pos, kind));
            }
        }
        self.open.retain(|k, _| shown.iter().any(|s| s.key == *k));
        self.phase.retain(|k, _| shown.iter().any(|s| s.key == *k));
        self.last.retain(|k, _| shown.iter().any(|s| s.key == *k));
        self.closing.retain(|c| !shown.iter().any(|s| s.key == c.0));
        let mut lights: Vec<(Vec3, [f32; 3])> = Vec::new();
        for c in self.closing.iter_mut() {
            c.1 += dt * 4.0;
            if c.1 < 1.0 {
                if let Some(t) = self.tunes.get(c.3 as usize) {
                    lights.push((c.2 + Vec3::Y * LIGHT_HEIGHT, light_color(t, None, 0.0, Some(c.1))));
                }
            }
        }
        self.closing.retain(|c| c.1 < 1.0);
        let plan = self.arrows.take();
        for s in shown {
            let timer = self.open.entry(s.key).or_insert(0.0);
            *timer = (*timer + dt * 2.0).min(1.0);
            let fade = *timer;
            self.last.insert(s.key, (s.pos, s.kind));
            let Some(t) = self.tunes.get(s.kind as usize).copied() else { continue };
            let dist = s.pos.distance(cam);
            fx.play_tinted(s.key, mcla_formats::hash::joaat(FLARE), s.pos, t.particle, t.shadow, fade, dist, &[("Height", height_evo(s.height))]);
            fx.set_arrow_on(s.key, plan.as_ref().map_or(false, |p| p.on.contains(&s.key)));
            if has_column(s.kind) {
                let time = time_evo(fx.hour);
                fx.play_tinted(s.key ^ 0x00c0_1000_0000_0000, mcla_formats::hash::joaat(COLUMN), s.pos, t.column, t.shadow, 1.0, dist, &[("time", time)]);
            }
            // the light: fading in while opening (timer below 1), then flickering
            let opening = (fade < 1.0).then_some(fade);
            let r = self.rand01();
            let ph = self.phase.entry(s.key).or_insert(0.0);
            if opening.is_some() {
                *ph = 0.0;
            }
            let col = light_color(&t, opening, *ph, None);
            if opening.is_none() {
                *ph = flicker_step(&t, *ph, r, dt);
            }
            lights.push((s.pos + Vec3::Y * LIGHT_HEIGHT, col));
        }
        // (the dynamic lights' fourth slot: the enabled checkpoint light nearest the camera,
        // within 500 m - `F-62430`; enabled = its colour is not black, `F-27408`)
        self.light = lights
            .into_iter()
            .filter(|(p, c)| c.iter().any(|v| *v != 0.0) && p.distance_squared(cam) < 250_000.0)
            .min_by(|a, b| a.0.distance_squared(cam).partial_cmp(&b.0.distance_squared(cam)).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(p, c)| (p, [c[0] * LIGHT_GAIN, c[1] * LIGHT_GAIN, c[2] * LIGHT_GAIN]));
        // (comparison aids: MCLA_CP_NO_LIGHT=1 / MCLA_CP_NO_ARROWS=1 leave the light / the arrows out)
        if std::env::var("MCLA_CP_NO_LIGHT").is_ok() {
            self.light = None;
        }
        // (MCLA_CP_LIGHT_CAM=1: the light hung at the camera - to see it on the road ahead)
        if std::env::var("MCLA_CP_LIGHT_CAM").is_ok() {
            self.light = self.light.map(|(_, c)| (cam, c));
        }
        // (MCLA_CP_LIGHT_X=<k>: the light k times as strong - to find it in a picture)
        if let Some(k) = std::env::var("MCLA_CP_LIGHT_X").ok().and_then(|v| v.parse::<f32>().ok()) {
            self.light = self.light.map(|(p, c)| (p, [c[0] * k, c[1] * k, c[2] * k]));
        }
        let plan = plan.filter(|_| std::env::var("MCLA_CP_NO_ARROWS").is_err());
        if plan.is_none() {
            for s in shown {
                fx.set_arrow_on(s.key, false);
            }
        }
        // (MCLA_CP_LOG=1: the light and the arrows' plan, once a second)
        if std::env::var("MCLA_CP_LOG").is_ok() && (self.clock * 60.0) as u64 % 60 == 0 {
            log::info!("checkpoint light {:?}; arrows {:?}", self.light, plan.as_ref().map(|p| (p.from, p.to, p.on.len(), arrow_angles(p.from, p.to, cam))));
        }
        // the arrows' target: turned by the camera and the time
        if let Some(p) = plan {
            let (z, x) = arrow_angles(p.from, p.to, cam);
            fx.arrow_rt = [z, x, arrow_frame(self.clock) as f32];
            self.clock += dt;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DATA: &str = "D:/MCLA/extracted/cache";

    #[test]
    fn round_checkpoint_reach() {
        let c = CpSpec::with_radius(Vec3::new(0.0, 10.0, 0.0), 10.0);
        // standing 9.9 m off: in; 10.1 m: out (the way is a point: no 1.2)
        let p = Vec3::new(9.9, 10.0, 0.0);
        assert!(c.hit(p, p));
        let p = Vec3::new(10.1, 10.0, 0.0);
        assert!(!c.hit(p, p));
        // driving past 11.5 m off: the nearest point lies inside the way - 1.2 x 10 = 12 m
        assert!(c.hit(Vec3::new(11.5, 10.0, 3.0), Vec3::new(11.5, 10.0, -3.0)));
        assert!(!c.hit(Vec3::new(12.5, 10.0, 3.0), Vec3::new(12.5, 10.0, -3.0)));
        // coming closer, not yet past: the way's end is nearest - the plain radius
        assert!(!c.hit(Vec3::new(11.0, 10.0, -1.0), Vec3::new(11.0, 10.0, -3.0)));
        // heights: 3 m below to the activation height (50) above
        let at = |y: f32| Vec3::new(1.0, 10.0 + y, 0.0);
        assert!(c.hit(at(-2.9), at(-2.9)));
        assert!(!c.hit(at(-3.1), at(-3.1)));
        assert!(c.hit(at(49.0), at(49.0)));
        assert!(!c.hit(at(50.5), at(50.5)));
        let low = CpSpec { act_height: 4.0, ..c };
        assert!(!low.hit(at(5.0), at(5.0)));
    }

    #[test]
    fn line_checkpoint_crossing() {
        // a line across the road along x from -10 to 10 at z = 0
        let c = CpSpec { second: Some(Vec3::new(10.0, 0.0, 0.0)), ..CpSpec::at(Vec3::new(-10.0, 0.0, 0.0)) };
        assert_eq!(c.draw_pos(), Vec3::ZERO);
        assert!((c.gate_offset().length() - 10.0).abs() < 1e-4);
        // across it
        assert!(c.hit(Vec3::new(0.0, 0.5, 1.0), Vec3::new(0.0, 0.5, -1.0)));
        // 20 % beyond its ends still counts (u up to 1.2: x = 14), further not
        assert!(c.hit(Vec3::new(13.5, 0.5, 1.0), Vec3::new(13.5, 0.5, -1.0)));
        assert!(!c.hit(Vec3::new(14.5, 0.5, 1.0), Vec3::new(14.5, 0.5, -1.0)));
        // short of it - but across the copy 10 m before it: counts
        assert!(c.hit(Vec3::new(0.0, 0.5, -9.0), Vec3::new(0.0, 0.5, -11.0)));
        assert!(!c.hit(Vec3::new(0.0, 0.5, -14.0), Vec3::new(0.0, 0.5, -16.0)));
        // along it (parallel): never
        assert!(!c.hit(Vec3::new(5.0, 0.5, 0.0), Vec3::new(-5.0, 0.5, 0.0)));
        // over it on a bridge 60 m up: no; 3 m under it: no
        assert!(!c.hit(Vec3::new(0.0, 60.0, 1.0), Vec3::new(0.0, 60.0, -1.0)));
        assert!(!c.hit(Vec3::new(0.0, -3.5, 1.0), Vec3::new(0.0, -3.5, -1.0)));
    }

    /// The original's line-line intersection (F-66886) and segment test (F-95591), run offline by
    /// `tools/checkpoint_cross_reference.py` on 3200 generated cases x flag 0 / 1 (axis-aligned, parallel, threshold values,
    /// random, car-step moves): the port equals them bit for bit. Evidence: docs/reference/checkpoints/verify_small_evidence.md.
    #[test]
    fn cross_matches_the_original() {
        let text = include_str!("../tests/data/cross_reference.txt");
        let (mut n, mut hits) = (0, 0);
        for line in text.lines() {
            let p: Vec<&str> = line.split(' ').collect();
            let h = |i: usize| u32::from_str_radix(p[i], 16).unwrap();
            let f = |i: usize| f32::from_bits(h(i));
            let (fnc, flag): (u32, u32) = (p[0].parse().unwrap(), p[1].parse().unwrap());
            let v = |x: f32, z: f32| Vec3::new(x, 0.0, z);
            let (a0, a1, b0, b1) = (v(f(2), f(3)), v(f(4), f(5)), v(f(6), f(7)), v(f(8), f(9)));
            let ret: u32 = p[10].parse().unwrap();
            let (ot, ou) = (h(11), h(12));
            let got = match (fnc, flag) {
                (0, 0) => cross_xz(a0, a1, b0, b1),
                // the one-way flag: the hangout entry's `meet` (u along the move, t along the line)
                (0, _) => crate::hangoutin::meet_xz(a0, a1, b0, b1),
                (_, 0) => cross_xz(a0, a1, b0, b1).filter(|(t, u)| (0.0..=1.0).contains(t) && (0.0..=1.0).contains(u)),
                (_, _) => crate::hangoutin::crossed_xz(b0, b1, a0, a1),
            };
            assert_eq!(got.is_some(), ret != 0, "fn {fnc} flag {flag} case {line}");
            if let Some((t, u)) = got {
                hits += 1;
                assert_eq!((t.to_bits(), u.to_bits()), (ot, ou), "fn {fnc} flag {flag} case {line}");
            }
            n += 1;
        }
        assert_eq!(n, 4 * 3200);
        assert!(hits > 4000, "{hits} crossings compared");
    }

    /// The constructor (F-26586), the member registration (F-15093), the tune folder (F-19793), the arrow look-at setter
    /// (F-27552) and the two light wrappers (F-63326, F-39031), run offline by `tools/checkpoint_small_reference.py`
    /// (vector mode of `tools/recomp_closure.py`), against the port. Evidence: docs/reference/checkpoints/verify_small_evidence.md.
    #[test]
    fn small_functions_match_the_original() {
        let text = include_str!("../tests/data/checkpoint_small_reference.txt");
        let hex = |x: &str| u32::from_str_radix(x, 16).unwrap();
        // the port's tune fields at the object offsets the original uses (3 floats for the colour triples, 4 for the discs)
        let fields = |t: &CpTune| -> Vec<(usize, Vec<f32>)> {
            vec![
                (16, t.light1.to_vec()), (32, t.light2.to_vec()), (48, t.particle.to_vec()), (64, t.column.to_vec()),
                (80, t.shadow.to_vec()), (96, t.glow.to_vec()), (112, t.inner1.to_vec()), (128, t.inner2.to_vec()),
                (144, t.outer1.to_vec()), (160, t.outer2.to_vec()), (176, vec![t.icon_scale]), (180, vec![t.icon_height]),
                (192, t.icon_color.to_vec()), (208, vec![t.flicker_random]), (212, vec![t.flicker_speed]),
                (216, vec![t.color_influence]), (220, vec![t.opacity_influence]), (224, vec![t.opacity]),
            ]
        };
        let (mut regs, mut looks, mut lpos, mut lcol) = (0, 0, 0, 0);
        let mut lpos_refold_differs = 0;
        let mut ctor_seen = false;
        for line in text.lines() {
            let p: Vec<&str> = line.split(' ').collect();
            match p[0] {
                "CTOR" => {
                    ctor_seen = true;
                    let bytes: Vec<u8> = p[1..].iter().map(|b| u8::from_str_radix(b, 16).unwrap()).collect();
                    assert_eq!(bytes.len(), 232);
                    let mut modelled = vec![false; 232];
                    for (off, vals) in fields(&CpTune::default()) {
                        for (i, v) in vals.iter().enumerate() {
                            let w = &bytes[off + 4 * i..off + 4 * i + 4];
                            assert_eq!(v.to_bits(), u32::from_be_bytes([w[0], w[1], w[2], w[3]]), "constructor field at +{off} lane {i}");
                            modelled[off + 4 * i..off + 4 * i + 4].fill(true);
                        }
                    }
                    // everything the constructor writes between +16 and +232 is a modelled field (the unwritten bytes stay 0xaa)
                    for (i, b) in bytes.iter().enumerate().skip(16) {
                        assert!(modelled[i] || *b == 0xaa, "constructor wrote +{i} which the port does not model");
                    }
                }
                "REG" => {
                    // type 8 = three floats, 9 = four, 6 = one: the port's parser must put the values into exactly that field
                    let (ty, name, off): (usize, &str, usize) = (p[1].parse().unwrap(), p[2], p[3].parse().unwrap());
                    assert_eq!((p[4], p[5]), ("1", "0"));
                    let n = match ty { 8 => 3, 9 => 4, 6 => 1, _ => panic!("type {ty}") };
                    let base = CpTune::default();
                    let mut t = base.clone();
                    let vals: Vec<f32> = (0..n).map(|i| 11.0 + i as f32 + off as f32 * 0.001).collect();
                    let text: Vec<String> = vals.iter().map(|v| format!("{v:?}")).collect();
                    parse_tune(&format!("{name} {}{}", text.join(" "), char::from(10)), &mut t);
                    for ((o, got), (_, was)) in fields(&t).into_iter().zip(fields(&base)) {
                        if o == off {
                            assert_eq!(got.iter().map(|v| v.to_bits()).collect::<Vec<_>>(), vals.iter().map(|v| v.to_bits()).collect::<Vec<_>>(), "{name}");
                        } else {
                            assert_eq!(got, was, "{name} changed the field at +{o}");
                        }
                    }
                    regs += 1;
                }
                "DIR" if p[1] == "0" => assert_eq!(p[2], TUNE_DIR),
                "DIR" => assert_eq!(p[2], "t://mc4//assets//tune//checkpoint"),
                "LOOK" => {
                    let (x, z) = (f32::from_bits(hex(p[1])), f32::from_bits(hex(p[2])));
                    let flag = p[3] == "01";
                    let got = look_at_of(x, z);
                    assert_eq!(got.is_some(), flag, "look-at ({x}, {z})");
                    if let Some(v) = got {
                        assert_eq!([v.x.to_bits(), v.y.to_bits(), v.z.to_bits()], [hex(p[4]), hex(p[5]), hex(p[6])]);
                    }
                    looks += 1;
                }
                "LPOS" => {
                    // the wrapper adds exactly 3.0 to y and keeps the other lanes; the port folds this 3 m and the caller's
                    // 0.25 m into one LIGHT_HEIGHT = 3.25 (`(y + 0.25) + 3` vs `y + 3.25`: counted below)
                    let w: Vec<u32> = p[1..].iter().map(|x| hex(x)).collect();
                    let y = f32::from_bits(w[1]);
                    assert_eq!([w[4], w[5], w[6], w[7]], [w[0], (y + 3.0).to_bits(), w[2], w[3]]);
                    if ((y + 0.25) + 3.0).to_bits() != (y + LIGHT_HEIGHT).to_bits() {
                        lpos_refold_differs += 1;
                    }
                    lpos += 1;
                }
                "LCOL" => {
                    let w: Vec<u32> = p[1..].iter().map(|x| hex(x)).collect();
                    for i in 0..4 {
                        assert_eq!(w[4 + i], (f32::from_bits(w[i]) * LIGHT_GAIN).to_bits(), "colour lane {i}");
                    }
                    lcol += 1;
                }
                other => panic!("unknown record {other}"),
            }
        }
        assert!(ctor_seen);
        assert_eq!((regs, looks, lpos, lcol), (18, 49, 302, 302));
        eprintln!("light height: {lpos_refold_differs} of {lpos} cases differ between (y + 0.25) + 3 and y + 3.25");
    }

    #[test]
    fn ahead_point_is_two_metres_along_the_velocity() {
        assert_eq!(ahead_point(Vec3::ONE, Vec3::new(0.0, 0.0, -30.0)), Vec3::new(1.0, 1.0, -1.0));
        assert_eq!(ahead_point(Vec3::ONE, Vec3::ZERO), Vec3::ONE);
    }

    #[test]
    fn looks_of_an_ordered_race() {
        // a sprint of 5: cp x 4, then finalcp
        assert_eq!((0..5).map(|i| ordered_kind(i, 5, 0, 1)).collect::<Vec<_>>(), vec![0, 0, 0, 0, 2]);
        // a circuit of 3 laps: the end of laps 1 and 2 finallapcp, of lap 3 finalcp
        assert_eq!(ordered_kind(3, 4, 0, 3), FINAL_LAP);
        assert_eq!(ordered_kind(3, 4, 1, 3), FINAL_LAP);
        assert_eq!(ordered_kind(3, 4, 2, 3), FINAL);
        assert_eq!(ordered_kind(0, 1, 0, 1), LIGHT_COLUMN);
        assert!(has_column(FINAL) && has_column(LIGHT_COLUMN) && has_column(CTF_PICKUP_NOTEAM) && !has_column(CP) && !has_column(FINAL_LAP) && !has_column(UNORDERED));
    }

    #[test]
    fn gps_goes_to_the_nearest_open_one_in_an_unordered_race() {
        let cps = [Vec3::new(100.0, 0.0, 0.0), Vec3::new(10.0, 0.0, 0.0), Vec3::new(50.0, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0)];
        // (the finish, nearest of all, is not open yet)
        assert_eq!(gps_target(&cps, 0, true, Vec3::ZERO), Some(cps[1]));
        assert_eq!(gps_target(&cps, 0, false, Vec3::ZERO), Some(cps[0]));
        assert_eq!(gps_target(&cps, 3, true, Vec3::ZERO), Some(cps[3]));
        assert_eq!(gps_target(&cps, 4, true, Vec3::ZERO), None);
        let s = [CpSpec::at(Vec3::new(5.0, 1.0, 5.0))];
        assert!(spec_at(&s, Vec3::new(5.0, 1.2, 5.0)).is_some());
        assert!(spec_at(&s, Vec3::new(5.5, 1.2, 5.0)).is_none());
    }

    #[test]
    fn evolution_values_and_blends() {
        use mcla_formats::rage::ptfx::{EvoGroup, EvoProp, Key, Keyframe};
        assert_eq!(height_evo(100.0), 1.0);
        assert!((height_evo(20.0) - 1.0 / 3.0).abs() < 1e-6);
        assert_eq!(height_evo(5.0), 0.0);
        assert_eq!(time_evo(12.0), 1.0);
        assert_eq!(time_evo(0.0), 0.0);
        assert!((time_evo(18.0) - 0.5).abs() < 1e-6 && (time_evo(6.0) - 0.5).abs() < 1e-6);
        let kf = |v: [f32; 4]| Keyframe { keys: vec![Key { t: 0.0, v, d: [0.0; 4] }] };
        let base = [10.0, 10.0, 0.0, 0.0];
        // mode 1 (the flare): base + sum of weight x (value - base)
        let g = EvoGroup { names: vec!["LOD".into(), "Height".into()], props: vec![EvoProp { list: 0, prop: 0, mode: 1, overrides: vec![(0, kf([4.0, 4.0, 0.0, 0.0])), (1, kf([12.0, 12.0, 0.0, 0.0]))] }] };
        assert_eq!(g.evolve(0, 0, base, 0.5, &[0.0, 1.0]), [12.0, 12.0, 0.0, 0.0]);
        assert_eq!(g.evolve(0, 0, base, 0.5, &[1.0, 1.0]), [6.0, 6.0, 0.0, 0.0]);
        assert_eq!(g.evolve(0, 1, base, 0.5, &[1.0, 1.0]), base);
        // mode 0 (the column's alpha): the mean of the lerps of the evolutions that are on
        let c = EvoGroup { names: vec!["alpha".into(), "time".into()], props: vec![EvoProp { list: 3, prop: 19, mode: 0, overrides: vec![(0, kf([0.0, 0.0, 1.0, 1.0])), (1, kf([1.0; 4]))] }] };
        let a = [0.06, 0.06, 1.0, 1.0];
        assert_eq!(c.evolve(3, 19, a, 0.0, &[0.0, 0.0]), a);
        assert_eq!(c.evolve(3, 19, a, 0.0, &[0.0, 1.0]), [1.0; 4]);
        let half = c.evolve(3, 19, a, 0.0, &[0.0, 0.5]);
        assert!((half[0] - 0.53).abs() < 1e-5);
    }

    #[test]
    fn projected_arrows_turn_with_the_camera() {
        use std::f32::consts::PI;
        let from = Vec3::ZERO;
        let to = Vec3::new(100.0, 0.0, 0.0);
        // seen from behind (the camera on the far side from where they point): turned by 3 pi / 2,
        // tipped by 1.9 - pointing up the target, away into the picture
        let (z, x) = arrow_angles(from, to, Vec3::new(-30.0, 0.0, 0.0));
        assert!((z - 1.5 * PI).rem_euclid(2.0 * PI).min((1.5 * PI - z).rem_euclid(2.0 * PI)) < 1e-5 && (x - 1.9).abs() < 1e-5, "{z} {x}");
        // from the side: flat (pi), and pointing to the camera's left / right
        let (z, x) = arrow_angles(from, to, Vec3::new(0.0, 0.0, -30.0));
        assert!((z - PI).abs() < 1e-5 && (x - PI).abs() < 1e-5, "{z} {x}");
        let (z, _) = arrow_angles(from, to, Vec3::new(0.0, 0.0, 30.0));
        assert!(z.abs() < 1e-5, "{z}");
        // (the vectors are made unit in 3D: a camera high above tips them less)
        let (_, x) = arrow_angles(from, to, Vec3::new(-30.0, 30.0, 0.0));
        let k = 0.5f32.sqrt();
        assert!((x - ((1.0 - k) * PI + 1.9 * k)).abs() < 1e-5);
        // the textures: anim_1, 2, 3 at 4 a second
        assert_eq!([0.0, 0.2, 0.26, 0.5, 0.74, 0.76, 3.0].map(arrow_frame), [1, 1, 2, 3, 3, 1, 1]);
    }

    #[test]
    fn checkpoint_light_fades_and_flickers() {
        let t = CpTune { light1: [1.0, 0.8, 0.6], light2: [0.6, 0.2, 0.0], flicker_random: 39.0, flicker_speed: 39.35, ..CpTune::default() };
        let near = |a: [f32; 3], b: [f32; 3]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-6);
        assert!(near(light_color(&t, Some(0.5), 0.0, None), [0.5, 0.4, 0.3]));
        assert!(near(light_color(&t, None, 0.0, Some(0.25)), [0.45, 0.15, 0.0]));
        // open: half way between the two at phase 0, a little further on at pi (sin(pi / 8))
        let mid = light_color(&t, None, 0.0, None);
        assert!((mid[0] - 0.8).abs() < 1e-6 && (mid[1] - 0.5).abs() < 1e-6);
        let top = light_color(&t, None, std::f32::consts::PI, None);
        let k = ((std::f32::consts::PI / 8.0).sin() + 1.0) * 0.5;
        assert!((top[0] - (1.0 - 0.4 * k)).abs() < 1e-6);
        // the phase runs at 39.35 .. 78.35 a second and wraps at pi
        let p = flicker_step(&t, 3.0, 0.0, 1.0 / 60.0);
        assert!((p - (3.0 + 39.35 / 60.0 - std::f32::consts::PI)).abs() < 1e-5);
        assert!((flicker_step(&t, 0.0, 1.0, 0.01) - 0.7835).abs() < 1e-5);
        assert_eq!((LIGHT_GAIN, LIGHT_HEIGHT, LIGHT_INTENSITY), (4.0, 3.25, 40.0));
    }

    /// The low smoke's shader variables (the rule's `rmPtfxShaderVar`s): the projected map's values.
    #[test]
    #[ignore]
    fn arrow_shader_vars_from_the_disc() {
        let res = crate::assets::load_resource_file(&Path::new(DATA).join("resources/rmptfx/fxlists/MC4.r107")).unwrap();
        let fx = mcla_formats::rage::ptfx::FxList::parse(&res).unwrap();
        let r = fx.particle_rules.iter().find(|r| r.name == "rage_flare_smoke").unwrap();
        let s = r.sprite.as_ref().unwrap();
        assert_eq!((s.shader.as_str(), s.technique.as_str()), ("mc4_rmptfx_projected", "draw"));
        let v = |n: &str| s.vars.iter().find(|(k, _)| k == n).unwrap().1;
        assert_eq!(v("ProjMapOffset"), [0.0, 5.0, 0.0, 0.0]);
        assert_eq!(v("ProjMapBounds"), [4.0, 5.0, 0.0, 0.0]);
        assert_eq!((v("ProjMapTile")[0], v("ProjMapRate")[0], v("NightArrowBoost")[0]), (1.0, 1.0, 0.0));
        assert_eq!(v("ArrowColorTint"), [1.0, 1.0, 1.0, 0.0]);
        let p = fx.particle_rules.iter().find(|r| r.name == "rage_flare_smoke_plume").unwrap();
        assert_eq!(p.sprite.as_ref().unwrap().technique, "draw_noarrow");
        // (both smokes: no random-colour bit - their vertices carry ColorKF x the tint, as the
        // original's vertex buffer shows: [A, 72, 51, 7] for the low smoke with cp's tint)
        for q in [r, p] {
            assert_eq!(q.flags, 0x2d28_5388);
            assert_eq!(q.flags & mcla_formats::rage::ptfx::flag::RANDOM_COLOR, 0);
        }
        let k = r.color.eval(0.5)[0];
        assert_eq!(((k * 255.0).floor(), (k * 0.713_725_5 * 255.0).floor(), (k * 0.098_039_22 * 255.0).floor()), (72.0, 51.0, 7.0));
        assert_eq!((p.color.eval(1.0)[0] * 255.0).floor(), 52.0);
        // (the effect's own ZoomKF: 39.07 % on every particle's size and on the domains - the
        // original's low smoke measured at ZoomKF x SizeKFOT x SizeKF, residual 0)
        let e = &fx.effects[fx.effect("checkpoint_ctf_generic").unwrap()];
        let z = e.zoom.eval(0.5);
        assert!((z[0] - 39.06976).abs() < 1e-3 && (z[1] - 39.06976).abs() < 1e-3, "{:?}", z);
        // (the programs drawn as the original's: the flare's fire lit (mode 4), the sparks unlit
        // (mode 5), the column its own mode)
        let mode = |n: &str| { let r = fx.particle_rules.iter().find(|r| r.name == n).unwrap(); crate::particles::sprite_mode(r.sprite.as_ref().unwrap(), r.render.blend_set) };
        assert_eq!((mode("cp_flare_fire"), mode("checkpoint_sparks"), mode("light_column")), (4.0, 5.0, 0.0));
        // (its cull sphere - the lit sprites' light picker works at its centre: 68 m straight up, x zoom)
        assert_eq!((e.cull_offset, e.cull_radius), ([0.0, 68.0, 0.0], 72.0));
        // (the draw order: events in the timeline's order (no SortEvents), the plume's emitter before
        // the low smoke that carries the arrows - so the arrows are drawn over the plume; both
        // sort their particles by a random key - docs/reference/checkpoints/arrow_draw_order.md)
        assert!(!e.sort_events);
        let rules: Vec<&str> = e.events.iter().filter_map(|ev| match &ev.kind { mcla_formats::rage::ptfx::EventKind::Emitter { particle: Some(pi), .. } => Some(fx.particle_rules[*pi].name.as_str()), _ => None }).collect();
        let at = |n: &str| rules.iter().position(|r| *r == n).unwrap();
        assert!(at("rage_flare_smoke_plume") < at("rage_flare_smoke"), "{:?}", rules);
        for n in ["rage_flare_smoke", "rage_flare_smoke_plume"] {
            let f = fx.particle_rules.iter().find(|r| r.name == n).unwrap().flags;
            assert!(f & mcla_formats::rage::ptfx::flag::SORT_PARTICLES != 0 && f & mcla_formats::rage::ptfx::flag::SORT_RANDOM != 0, "{} {:#x}", n, f);
        }
        // (no effect on the disc sets SortEvents: the timeline's order is every effect's)
        assert_eq!(fx.effects.iter().filter(|q| q.sort_events).count(), 0);
    }

    #[test]
    #[ignore]
    fn tunes_from_the_disc() {
        let t = load_tunes(DATA);
        assert_eq!(t.len(), 18);
        // cp: orange smoke, no file for fillercp (constructor values)
        assert_eq!(t[0].particle, [1.0, 0.71372551, 0.098039217]);
        assert_eq!(t[2].column, [0.4, 0.0, 0.0]);
        assert_eq!(t[16], CpTune::default());
        assert_eq!(t[4].inner1, [0.15, 0.95, 0.9, 0.25]);
        assert_eq!(t[3].outer1, [0.95, 0.95, 0.95, 0.25]);
    }

    /// MCLA_CP_RACE=<race id>: print a race's checkpoints as its script sets them up.
    #[test]
    #[ignore]
    fn print_race_specs() {
        let Ok(id) = std::env::var("MCLA_CP_RACE") else { return };
        let defs = crate::races::load(DATA);
        let d = defs.iter().find(|d| d.id.ends_with(&id)).expect("race");
        for (k, s) in script_specs(DATA, &d.id, d.checkpoints.first().copied()).unwrap_or_default().iter().enumerate() {
            eprintln!("{k}: pos {:?} second {:?} draw {:?} radius {} height {} act {} speed {}", s.pos, s.second, s.draw_pos(), s.radius, s.height, s.act_height, s.max_speed);
        }
    }

    /// Every race script's checkpoint tables read in the expected layout (radii, heights,
    /// lines); prints the spread of values.
    #[test]
    #[ignore]
    fn script_tables_of_every_race() {
        let defs = crate::races::load(DATA);
        let (mut ok, mut none, mut lines, mut cps) = (0, 0, 0, 0);
        let mut radii: Vec<f32> = Vec::new();
        let mut acts: Vec<f32> = Vec::new();
        for d in defs.iter().filter(|d| matches!(d.race_type.as_str(), "Ordered" | "Unordered") && !d.checkpoints.is_empty()) {
            match script_specs(DATA, &d.id, d.checkpoints.first().copied()) {
                Some(s) => {
                    ok += 1;
                    cps += s.len();
                    lines += s.iter().filter(|c| c.second.is_some()).count();
                    radii.extend(s.iter().map(|c| c.radius));
                    acts.extend(s.iter().map(|c| c.act_height));
                    if s.len() != d.checkpoints.len() {
                        eprintln!("{}: script {} checkpoints, file {}", d.id, s.len(), d.checkpoints.len());
                    }
                }
                None => {
                    none += 1;
                    eprintln!("no tables: {}", d.id);
                }
            }
        }
        radii.sort_by(|a, b| a.partial_cmp(b).unwrap());
        acts.sort_by(|a, b| a.partial_cmp(b).unwrap());
        eprintln!("{ok} scripts read, {none} not; {cps} checkpoints, {lines} lines; radius {:?} .. {:?} median {:?}; act height {:?} .. {:?}", radii.first(), radii.last(), radii.get(radii.len() / 2), acts.first(), acts.last());
        assert!(ok > 0);
    }
}

/// The GPS destination (`UILogic_SetGPSDestination`): an ordered race's next checkpoint; in an
/// unordered race the open checkpoint nearest the player by straight distance
/// (`CheckpointManager_FindClosestOpenCheckpoint`, `F-98939`) - the finish once it is
/// the only one left.
pub fn gps_target(checkpoints: &[Vec3], next: usize, unordered: bool, player: Vec3) -> Option<Vec3> {
    let n = checkpoints.len();
    if next >= n {
        return None;
    }
    if !unordered || next + 1 >= n {
        return Some(checkpoints[next]);
    }
    checkpoints[next..n - 1].iter().copied().min_by(|a, b| a.distance_squared(player).partial_cmp(&b.distance_squared(player)).unwrap_or(std::cmp::Ordering::Equal))
}
