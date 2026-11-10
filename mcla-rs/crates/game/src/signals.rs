//! Traffic signals as the original runs them (read from the executable: `mcRnIntersectionAmbient`
//! and `mcTrafficLightInst`, write-up in `D:\MCLA\work\map\signal_rule.md`).
//!
//! * The junctions are the road network's own (`NavJunction`: centre, box, one outward
//!   direction per arm), 339 with arms.
//! * A phase is one arm: only the current arm has green - 6.5 s green, 2 s yellow, 1.5 s all
//!   red, then the next arm. Every junction runs in step; there is no offset between them.
//! * A signal head belongs to the arm its z axis points along (it faces the drivers coming in
//!   on that arm). Heads are bound to arms greedily by that dot product; a head no junction
//!   takes shows green. Every lamp of a head shows the same state.
//!
//! * Each junction keeps its own state (`Signals::step`, the original's update F-86376):
//!   the current arm, the phase timer and a demand counter per arm. A car waiting at a red arm
//!   adds to that arm's demand every tick; when the phase is over the arm with the largest
//!   demand is next (arm 0 when nobody waits), and the switch waits - up to 5 s - while a car is
//!   still in the box. In a race every junction is red all round during the countdown, the
//!   first switch after the start goes to the player's arm, and a fast player gets priority at
//!   the junction ahead. The vehicles' copy is stepped; the props' copy takes its state
//!   (`sync_from`).
//!
//! Read only in part from the code, so approximate here: what counts as "in the box" for cars
//! that are not lane traffic (see `step`).
//! The lamp state reaches the prop shader through the instance scale (see `Props::update_signals`).

use glam::Vec3;
use hashbrown::HashMap;
use mcla_formats::rage::navsys::{NavJunction, NavSys};

const GREEN: f32 = 6.5;
const YELLOW: f32 = 2.0;
const ALL_RED: f32 = 1.5;
pub const PHASE: f32 = GREEN + YELLOW + ALL_RED;
/// the timer every junction starts with (all red, 1.5 s before its first green)
const START: f32 = GREEN + YELLOW;

/// Head that no junction takes.
pub const UNBOUND: u32 = u32::MAX;

#[derive(Clone, Copy, Debug)]
pub struct SignalHead {
    /// index into the prop instance buffer
    pub inst: u32,
    pub pos: Vec3,
    /// direction the lamps shine towards (unit, horizontal): the head's z axis
    pub facing: Vec3,
    /// its junction (`UNBOUND`: none - it shows green) and arm
    pub junction: u32,
    pub arm: u8,
    /// last state written to the instance buffer
    pub shown: u8,
}

#[derive(Clone, Debug)]
pub struct Junction {
    pub center: Vec3,
    /// outward unit direction of each arm (level)
    pub arms: Vec<Vec3>,
    /// reach of the box from its centre (m)
    pub radius: f32,
    /// the junction's +0x40 (its second box half extent): a refused car this close to the
    /// centre counts as waiting (F-16850)
    pub reach: f32,
    /// held by a race script: -1 free, -2 red all round, otherwise the arm held green
    pub forced: i8,
    /// the arm whose phase runs
    pub current: u8,
    /// seconds into the phase: green to 6.5, yellow to 8.5, red all round from there
    pub timer: f32,
    /// car-ticks waited at each arm since its last green
    pub demand: Vec<u32>,
}

fn level(v: Vec3) -> Vec3 {
    Vec3::new(v.x, 0.0, v.z).normalize_or(Vec3::X)
}

/// The vector unit's normalisation of the flattened vector as F-97162 does it for the arms and the point (compare
/// `lane_dot`): s = x x + z z, r0 = 1 / sqrt(s), one Newton step -(s r0 r0 - 1)(0.5 r0) + r0, times x and z; denormals flushed.
/// A zero vector gives NaN (no arm is greater than anything then).
pub(crate) fn unit_exact(v: Vec3) -> Vec3 {
    let ftz = |x: f32| if x.is_subnormal() { 0.0f32.copysign(x) } else { x };
    let (x, z) = (ftz(v.x), ftz(v.z));
    let s = ftz(ftz(x * x) + ftz(z * z));
    let r0 = ftz(1.0 / s.sqrt());
    let v9 = ftz(r0 * r0);
    let v8 = ftz(0.5 * r0);
    let v7 = -ftz(ftz(s * v9) - 1.0);
    let v6 = ftz(ftz(v7 * v8) + r0);
    Vec3::new(ftz(x * v6), 0.0, ftz(z * v6))
}

/// The lane green tests' arm . entry (F-78185 / F-16432): the entry flattened (y = 0) and normalised as the vector
/// unit does it (the three-lane dot, 1 / sqrt, one Newton step -(s y y - 1)(0.5 y) + y, denormals flushed), then
/// fma(a.x, n.x, fma(a.z, n.z, a.y n.y)). Compared with the image's cos 25 degrees, `LANE_GREEN_COS`.
pub(crate) fn lane_dot(a: Vec3, entry: Vec3) -> f32 {
    let ftz = |x: f32| if x.is_subnormal() { 0.0f32.copysign(x) } else { x };
    let (x, z) = (ftz(entry.x), ftz(entry.z));
    let s = ftz(ftz(x * x) + ftz(z * z));
    let r0 = ftz(1.0 / s.sqrt());
    let v9 = ftz(r0 * r0);
    let v8 = ftz(0.5 * r0);
    let v7 = -ftz(ftz(s * v9) - 1.0);
    let v6 = ftz(ftz(v7 * v8) + r0);
    let n = [ftz(x * v6), ftz(0.0 * v6), ftz(z * v6)];
    let f12 = a.y * n[1];
    let f7 = a.z.mul_add(n[2], f12);
    a.x.mul_add(n[0], f7)
}

/// cos 25 degrees as the image has it (0.9063078).
pub(crate) const LANE_GREEN_COS: f32 = f32::from_bits(0x3F68_03CA);

impl Junction {
    /// The arm a point (or, for `dir`, a direction from the centre) lies on.
    /// F-97162: the largest dot product, the first of equals (strict `>` from -FLT_MAX); 0 when there is no arm (the
    /// original gives -1 and its callers skip it).
    pub fn arm_towards(&self, dir: Vec3) -> usize {
        let d = unit_exact(dir);
        let (mut best, mut at) = (f32::MIN, 0);
        for (k, a) in self.arms.iter().enumerate() {
            let x = a.dot(d);
            if x > best {
                best = x;
                at = k;
            }
        }
        at
    }

    /// F-97162 with a point: the point with its y taken as 0 minus `mid`, the middle of the junction's box (its +0x50, `Signals::nav_mid`; its y stays), normalised as the vector
    /// unit does it (the three-lane dot (x x + y y) + z z, 1 / sqrt, one Newton step, denormals flushed), against every arm
    /// (`unit_exact` of the raw arm, as `build` holds them): the largest dot, the first of equals, strictly above -FLT_MAX; none
    /// without arms (the original's -1).
    pub fn arm_of_point(&self, mid: Vec3, p: Vec3) -> Option<usize> {
        let ftz = |x: f32| if x.is_subnormal() { 0.0f32.copysign(x) } else { x };
        let (x, y, z) = (ftz(p.x - mid.x), ftz(0.0 - mid.y), ftz(p.z - mid.z));
        let s = ftz(ftz(ftz(x * x) + ftz(y * y)) + ftz(z * z));
        let r0 = ftz(1.0 / s.sqrt());
        let v9 = ftz(r0 * r0);
        let v8 = ftz(0.5 * r0);
        let v7 = -ftz(ftz(s * v9) - 1.0);
        let r = ftz(ftz(v7 * v8) + r0);
        let n = [ftz(x * r), ftz(y * r), ftz(z * r)];
        let (mut best, mut at) = (f32::MIN, None);
        for (k, a) in self.arms.iter().enumerate() {
            let d = ftz(ftz(ftz(a.x * n[0]) + ftz(a.y * n[1])) + ftz(a.z * n[2]));
            if d > best {
                best = d;
                at = Some(k);
            }
        }
        at
    }

    /// State of an arm: 0 red, 1 yellow, 2 green. (`_t` is not used any more: the state is the
    /// junction's own.) Only the current arm and the timer count, as in the original's tests (F-23734 / F-23525 /
    /// F-59460, the lane tests F-78185 / F-16432 and the lamp's F-83493 read J +160 / +164 only): a forced arm is green because `step` makes it current
    /// with the timer at 0 every update, and is red all round while `step` holds the timer at 10 (a race's countdown,
    /// the red light race flag).
    pub fn arm_state(&self, arm: usize, _t: f32) -> u8 {
        arm_light(self.current as i64, self.timer, arm as i64)
    }

    /// May traffic enter a junction lane whose first two control points are P0, P1 (`entry` =
    /// P0 - P1, pointing back out along the street it comes from)? The original's test
    /// (F-78185 / F-16432): only while the phase is green (timer < 6.5) and the
    /// current arm lies within 25 degrees of the lane's entry, `lane_dot(arm, P0 - P1) > cos 25` (the image's 0.9063078).
    /// The car's own heading does not count: on a street that bends into the junction it can
    /// be far off the arm while the lane's first span still runs along it. 2 green, 1 yellow
    /// (the arm's yellow; traffic stops for it too), 0 red.
    /// (Like `arm_state`, J +160 / +164 only: a forced arm through `step`.)
    pub fn lane_state(&self, entry: Vec3) -> u8 {
        let (current, timer) = (self.current as usize, self.timer);
        match self.arms.get(current) {
            Some(a) if lane_dot(*a, entry) > LANE_GREEN_COS && timer < GREEN + YELLOW => {
                if timer < GREEN {
                    2
                } else {
                    1
                }
            }
            _ => 0,
        }
    }

    /// Signal state for traffic travelling along `heading` towards the junction at time `t`:
    /// the car is on the arm that points back the way it came.
    pub fn state_for(&self, heading: Vec3, t: f32) -> u8 {
        self.arm_state(self.arm_towards(-heading), t)
    }
}

/// The colour a head shows for the junction's arm `arm` (0 red, 1 yellow, 2 green), from the junction's current arm and phase timer
/// alone, as F-83493 chooses it: the head's index in the junction's list is the arm; green while it is the current arm and the timer
/// is below 6.5, yellow from 6.5 up to 8.5, red otherwise (tests/`original_signal_heads`). The original also forces green for every
/// head when the ambient manager's all-green byte is set, which only its online mode sets (not modelled).
pub fn arm_light(current: i64, timer: f32, arm: i64) -> u8 {
    if arm != current || timer >= GREEN + YELLOW {
        0
    } else if timer < GREEN {
        2
    } else {
        1
    }
}

#[derive(Default, Clone)]
pub struct Signals {
    pub junctions: Vec<Junction>,
    grid: HashMap<(i32, i32), Vec<u32>>,
    /// (junction, arm) of every traffic car the light refused since the last step (`wait`)
    waiting: Vec<(u32, u8)>,
    /// The red light race byte (F-12073 reads it): `RnIntersectionAmbient_SetRedLightRace(b)` (F-15914) stores
    /// `b != 0`, `..._GetRedLightRace()` (F-71155) returns it; nothing else writes it (the reset F-35280 leaves it).
    /// While set, `step` holds every junction red all round (before a forced arm) and a car of the bIsProxy pool
    /// is let through a red light (`light_go`). The five `<district>_redlight_hk1` scripts set it to 1 right after
    /// the warp to the start ('REDLIGHT', WAIT(150), then Race_SetTimer(-7000)) and to 0 once Race_GetTimer >= 0
    /// (the go). Port: `Game::alt_after_start` and `Game::update_race`.
    pub red_light_race: bool,
    /// per junction: its index in the road network's junction array (`NavJunction::index`), how the original's lookup
    /// F-56988 names it (`force`)
    pub nav_index: Vec<usize>,
    /// per junction: the middle of its box (`NavJunction::mid`, the +0x50 F-97162 measures a forced point from)
    pub nav_mid: Vec<Vec3>,
}

/// The riders' light (F-16850): may a car enter the junction lane? `green` = the lane's green test (F-78185, `lane_state`
/// 2); a car of the bIsProxy pool (car +500 bit 0x20) goes on red too while the red light race byte is set (F-12073);
/// then the box must not be held by anyone but itself (F-50959 with the car, asked only when it would go).
pub fn light_go(green: bool, proxy: bool, red_light_race: bool, blocked: impl FnOnce() -> bool) -> bool {
    (green || (proxy && red_light_race)) && !blocked()
}

const CELL: f32 = 96.0;

fn cell(p: Vec3) -> (i32, i32) {
    ((p.x / CELL).floor() as i32, (p.z / CELL).floor() as i32)
}

impl Signals {
    /// The middle of junction `ji`'s box (J +0x50, which F-97162 and the refused car's reach test measure from): `nav_mid`,
    /// else (junctions made without the road network) its centre.
    pub fn mid_of(&self, ji: usize) -> Vec3 {
        self.nav_mid.get(ji).copied().or_else(|| self.junctions.get(ji).map(|j| j.center)).unwrap_or_default()
    }

    /// The race events' intersection fire F-11036 (`vIntersectionCenter` = `center`, `vDirection` = `point`,
    /// `bGreenElseAllRed` = `green`): the junction whose box holds `center` (F-56988: `NavSys::junction_at`); nothing when
    /// there is none or when x, y and z of `point` are all zero (a float compare: -0 is zero, NaN is not), red all round
    /// included; otherwise the arm of `point` (F-97162, `Junction::arm_of_point`) is held green, or the junction red all round.
    /// (The original also does nothing while the command-line switch `-notriggeredintersections` is given, or for a junction
    /// without arms but its free flag; the port's junctions all have arms.) Returns whether a junction was found.
    pub fn force(&mut self, nav: Option<&NavSys>, center: Vec3, point: Vec3, green: bool) -> bool {
        let Some(ni) = nav.and_then(|n| n.junction_at(center)) else { return false };
        if point.x == 0.0 && point.y == 0.0 && point.z == 0.0 {
            return true;
        }
        let Some(k) = self.nav_index.iter().position(|&i| i == ni) else { return true };
        let mid = self.nav_mid.get(k).copied().unwrap_or_default();
        let Some(j) = self.junctions.get_mut(k) else { return true };
        j.forced = match j.arm_of_point(mid, point) {
            None => -1,
            Some(a) if green => a as i8,
            Some(_) => -2,
        };
        true
    }

    /// The original's reset (F-35280): nothing forced, arm 0, red all round for 1.5 s more,
    /// no demand - every junction in step.
    pub fn release(&mut self) {
        for j in self.junctions.iter_mut() {
            j.forced = -1;
            j.current = 0;
            j.timer = START;
            j.demand.iter_mut().for_each(|d| *d = 0);
        }
    }

    /// Take the state of another copy of the same junctions.
    pub fn sync_from(&mut self, other: &Signals) {
        for (j, o) in self.junctions.iter_mut().zip(other.junctions.iter()) {
            j.forced = o.forced;
            j.current = o.current;
            j.timer = o.timer;
        }
        self.red_light_race = other.red_light_race;
    }

    /// One tick of every junction. `cars`: position and velocity of the cars on the road;
    /// `player`: the player's car; `race`: (countdown left, race clock) while there is a race.
    ///
    /// Order as F-86376: countdown -> red all round; forced red; the red light race byte -> red all round; forced
    /// arm (held green);
    /// else the timer runs, and past 10 s the junction switches once its box is clear or 15 s
    /// are up - in the first 1.5 s of a race to the player's arm, otherwise to the arm with the
    /// largest demand, arm 0 if there is none.
    /// Demand: every traffic car the light refused since the last step and within the
    /// junction's reach adds one to the arm it stands on (`wait`, F-16850). Box held:
    /// `blocked[ji]` per junction = F-50959(J, 0) (`Vehicles::junction_blocked`: the
    /// player on one of its lanes within 3 m, or ambient cars on its rails).
    /// The player's demand (`demand`, F-42082 -> F-11273, the junction ahead of the
    /// player's lane found by `traffic::player_junction_record`): see `player_demand`.
    pub fn step(&mut self, dt: f32, blocked: &[bool], demand: Option<(usize, Vec3, bool)>, player: Option<(Vec3, Vec3)>, race: Option<(f32, f32)>) {
        for (ji, arm) in std::mem::take(&mut self.waiting) {
            if let Some(c) = self.junctions.get_mut(ji as usize).and_then(|j| j.demand.get_mut(arm as usize)) {
                *c = c.saturating_add(1);
            }
        }
        if let Some((ji, pos, prio)) = demand {
            self.player_demand(ji, pos, prio);
        }
        let red_light_race = self.red_light_race;
        let mids = &self.nav_mid;
        for (ji, j) in self.junctions.iter_mut().enumerate() {
            if race.map_or(false, |r| r.0 > 0.0) || j.forced == -2 || red_light_race {
                j.timer = PHASE;
                continue;
            }
            if j.forced >= 0 {
                j.current = j.forced as u8;
                j.timer = 0.0;
                continue;
            }
            j.timer += dt;
            if j.timer <= PHASE || (blocked.get(ji).copied().unwrap_or(false) && j.timer <= PHASE + 5.0) {
                continue;
            }
            j.timer = 0.0;
            let next = match (race, player) {
                // (F-97162 on the player's position, measured from the box middle +0x50)
                (Some((_, clock)), Some((pos, _))) if clock < 1.5 => j.arm_of_point(mids.get(ji).copied().unwrap_or(j.center), pos).unwrap_or(0),
                // (the first of the largest counters above 0, compared as signed words; 0 if none)
                _ => {
                    let (mut best, mut at) = (0i32, 0);
                    for (k, &d) in j.demand.iter().enumerate() {
                        if d as i32 > best {
                            best = d as i32;
                            at = k;
                        }
                    }
                    at
                }
            };
            j.current = next as u8;
            if let Some(c) = j.demand.get_mut(next) {
                *c = 0;
            }
        }
    }

    /// The road network's junctions, and for every signal head (position, z axis) the junction
    /// and arm it is bound to (`UNBOUND`: none). Binding as the original's `F-83643`: the
    /// heads round a junction's centre - within 0.66 of its half extent, counting a head's own
    /// reach of about 6.5 m - are matched to the arms greedily by the dot product of head axis
    /// and arm direction, one head an arm, the dot not negative, the head within 2.5 m of the
    /// junction's height.
    pub fn build(junctions: &[NavJunction], heads: &[(Vec3, Vec3)]) -> (Signals, Vec<(u32, u8)>) {
        const HEAD_REACH: f32 = 6.5;
        let mut of_head = vec![(UNBOUND, 0u8); heads.len()];
        let mut hgrid: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
        for (i, h) in heads.iter().enumerate() {
            hgrid.entry(cell(h.0)).or_default().push(i);
        }
        let mut out = Vec::with_capacity(junctions.len());
        for (ji, nj) in junctions.iter().enumerate() {
            let arms: Vec<Vec3> = nj.arms.iter().map(|a| if a.x == 0.0 && a.z == 0.0 { Vec3::X } else { unit_exact(*a) }).collect();
            let reach = nj.half[1].max(1.0) * 0.66 + HEAD_REACH;
            let mut cand: Vec<usize> = Vec::new();
            let (c0, c1) = (cell(nj.center - Vec3::splat(reach)), cell(nj.center + Vec3::splat(reach)));
            for cx in c0.0..=c1.0 {
                for cz in c0.1..=c1.1 {
                    for &hi in hgrid.get(&(cx, cz)).map(|v| v.as_slice()).unwrap_or(&[]) {
                        let h = &heads[hi];
                        if of_head[hi].0 == UNBOUND && (h.0.y - nj.center.y).abs() < 2.5 && Vec3::new(h.0.x - nj.center.x, 0.0, h.0.z - nj.center.z).length() <= reach {
                            cand.push(hi);
                        }
                    }
                }
            }
            let mut scores: Vec<(f32, usize, usize)> = Vec::new();
            for (a, dir) in arms.iter().enumerate() {
                for &hi in &cand {
                    let s = heads[hi].1.dot(*dir);
                    if s >= 0.0 {
                        scores.push((s, a, hi));
                    }
                }
            }
            scores.sort_by(|x, y| y.0.total_cmp(&x.0));
            let mut arm_done = vec![false; arms.len()];
            for (_, a, hi) in scores {
                if !arm_done[a] && of_head[hi].0 == UNBOUND {
                    arm_done[a] = true;
                    of_head[hi] = (ji as u32, a as u8);
                }
            }
            let demand = vec![0; arms.len()];
            out.push(Junction { center: nj.center, arms, radius: nj.half[0].max(nj.half[1]).clamp(6.0, 40.0), reach: nj.half[1], forced: -1, current: 0, timer: START, demand });
        }
        let mut grid: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
        for (i, j) in out.iter().enumerate() {
            grid.entry(cell(j.center)).or_default().push(i as u32);
        }
        (Signals { junctions: out, grid, waiting: Vec::new(), red_light_race: false, nav_index: junctions.iter().map(|j| j.index).collect(), nav_mid: junctions.iter().map(|j| j.mid).collect() }, of_head)
    }

    /// F-11273(J, pos, vel, 1, prio, hold = prio): the player adds demand to junction `ji`
    /// (the one ahead of its lane). The arm is the one nearest the player's bearing from the
    /// centre (F-97162). Not `prio` (free roam): +1 to that arm unless it is current.
    /// `prio` (a race running and above 10 mph): green (timer < 6.5) on the player's own arm
    /// -> held (timer 0); on another arm -> demand +1 and the timer to 6.5 (yellow at once);
    /// yellow -> demand +1, timer at least 8.25; red all round -> demand +1, timer at least 9.75.
    pub fn player_demand(&mut self, ji: usize, pos: Vec3, prio: bool) {
        let mid = self.mid_of(ji);
        let Some(j) = self.junctions.get_mut(ji) else { return };
        if j.arms.is_empty() {
            return;
        }
        // (F-97162 on the position, from the box middle +0x50)
        let arm = j.arm_of_point(mid, pos).unwrap_or(0);
        let add = |j: &mut Junction| {
            if let Some(c) = j.demand.get_mut(arm) {
                *c = c.saturating_add(1);
            }
        };
        if !prio {
            if j.current as usize != arm {
                add(j);
            }
        } else if j.timer < GREEN {
            if j.current as usize == arm {
                j.timer = 0.0;
            } else {
                add(j);
                j.timer = GREEN;
            }
        } else if j.timer < GREEN + YELLOW {
            add(j);
            j.timer = j.timer.max(GREEN + YELLOW - 0.25);
        } else {
            add(j);
            j.timer = j.timer.max(PHASE - 0.25);
        }
    }

    /// A traffic car at `pos` that the light of junction `ji` refused this frame (F-16850):
    /// within the junction's reach of the middle of its box (J +80 = +0x50, `mid_of`) it adds one to the demand of the arm it
    /// stands on (F-97162 on the position, from the same middle), at the next step.
    pub fn wait(&mut self, ji: usize, pos: Vec3) {
        let mid = self.mid_of(ji);
        let Some(j) = self.junctions.get(ji) else { return };
        // (F-16850: dz * dz + dx * dx fused, against the reach squared)
        let (dx, dz) = (mid.x - pos.x, mid.z - pos.z);
        if dz.mul_add(dz, dx * dx) < j.reach * j.reach && !j.arms.is_empty() {
            self.waiting.push((ji as u32, j.arm_of_point(mid, pos).unwrap_or(0) as u8));
        }
    }

    /// The junction a vehicle at `pos` travelling along `heading` is driving up to:
    /// (junction index, metres to its stop line, signal state).
    pub fn ahead(&self, pos: Vec3, heading: Vec3, t: f32) -> Option<(usize, f32, u8)> {
        let h = level(heading);
        let (cx, cz) = cell(pos);
        let mut best: Option<(usize, f32, u8)> = None;
        for dx in -1..=1 {
            for dz in -1..=1 {
                let Some(list) = self.grid.get(&(cx + dx, cz + dz)) else { continue };
                for &ji in list {
                    let j = &self.junctions[ji as usize];
                    let r = Vec3::new(j.center.x - pos.x, 0.0, j.center.z - pos.z);
                    if (j.center.y - pos.y).abs() > 7.0 {
                        continue;
                    }
                    let d = r.length();
                    if d > 90.0 || d < 1.0 {
                        continue;
                    }
                    let along = r.dot(h);
                    let side = (r - h * along).length();
                    // heading for the box, roughly along one of its arms
                    if along <= 0.0 || side > j.radius * 0.8 + 3.0 {
                        continue;
                    }
                    // (the original lets a lane in only when the green arm lies within 25
                    // degrees of it: a car coming in between two arms has no green)
                    let arm = j.arm_towards(-h);
                    let state = if j.arms[arm].dot(-h) > 0.9063 { j.arm_state(arm, t) } else { 0 };
                    let stop = along - (j.radius + 2.0);
                    if best.map_or(true, |b| stop < b.1) {
                        best = Some((ji as usize, stop, state));
                    }
                }
            }
        }
        best
    }

    /// The junction whose box holds `p` (a junction lane's control points): the nearest centre
    /// within its reach plus 12 m and 7 m of height.
    pub fn junction_at(&self, p: Vec3) -> Option<usize> {
        let (cx, cz) = cell(p);
        let mut best: Option<(usize, f32)> = None;
        for dx in -1..=1 {
            for dz in -1..=1 {
                let Some(list) = self.grid.get(&(cx + dx, cz + dz)) else { continue };
                for &ji in list {
                    let j = &self.junctions[ji as usize];
                    let d = Vec3::new(p.x - j.center.x, 0.0, p.z - j.center.z).length();
                    if (j.center.y - p.y).abs() < 7.0 && d < j.radius + 12.0 && best.map_or(true, |b| d < b.1) {
                        best = Some((ji as usize, d));
                    }
                }
            }
        }
        best.map(|b| b.0)
    }

    /// Junction whose box contains `pos`, if any.
    pub fn inside(&self, pos: Vec3) -> Option<usize> {
        let (cx, cz) = cell(pos);
        for dx in -1..=1 {
            for dz in -1..=1 {
                let Some(list) = self.grid.get(&(cx + dx, cz + dz)) else { continue };
                for &ji in list {
                    let j = &self.junctions[ji as usize];
                    if (j.center.y - pos.y).abs() < 7.0 && Vec3::new(j.center.x - pos.x, 0.0, j.center.z - pos.z).length() < j.radius - 2.0 {
                        return Some(ji as usize);
                    }
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cross() -> Junction {
        Junction { center: Vec3::ZERO, arms: vec![Vec3::X, Vec3::Z, -Vec3::X, -Vec3::Z], radius: 12.0, reach: 12.0, forced: -1, current: 0, timer: START, demand: vec![0; 4] }
    }

    /// F-97162 run offline on the original's code (tools/arm_towards_reference.py -> testdata/arm_towards_ref.txt): 6,000
    /// junctions of 0 .. 6 raw arms (any length and slope, crossings with the point on a bisector, repeated arms) and a
    /// point; the arm index must be equal (the port's arms are the build's, `unit_exact` of the raw arm; -1 is 0).
    #[test]
    fn arm_towards_matches_the_original() {
        let h = |t: &str| f32::from_bits(u32::from_str_radix(t, 16).unwrap());
        let (mut n, mut bad) = (0, Vec::new());
        for line in include_str!("testdata/arm_towards_ref.txt").lines() {
            let line = line.replace('_', "");
            let (i, o) = line.split_once(" -> ").unwrap();
            let w: Vec<&str> = i.split_whitespace().collect();
            let k: usize = w[0].parse().unwrap();
            let v = |at: usize| Vec3::new(h(w[at]), h(w[at + 1]), h(w[at + 2]));
            let arms: Vec<Vec3> = (0..k).map(|a| unit_exact(v(1 + 4 * a))).collect();
            let j = Junction { center: Vec3::ZERO, arms, radius: 12.0, reach: 12.0, forced: -1, current: 0, timer: START, demand: vec![0; k] };
            let want: i32 = o.parse().unwrap();
            if j.arm_towards(v(1 + 4 * k)) as i32 != want.max(0) {
                bad.push(n);
            }
            n += 1;
        }
        assert_eq!(n, 6003);
        assert!(bad.is_empty(), "{} of {n} differ, first cases {:?}", bad.len(), &bad[..bad.len().min(12)]);
    }

    /// F-23734 / F-23525 / F-59460 (green, yellow, red test of arm i) recorded in the running original by
    /// the object-dump recorder (docs/reference/map/objdump_signals.md): the port's `arm_state` of a free
    /// junction (forced -1) gives the same answer for every recorded call.
    #[test]
    fn arm_tests_match_the_original() {
        let text = include_str!("testdata/signals_arm_ref.txt");
        let mut n = [0usize; 3];
        for l in text.lines() {
            let t: Vec<&str> = l.split(' ').collect();
            let (cur, timer, arm, allgreen, ret): (i32, u32, usize, u8, u8) =
                (t[1].parse().unwrap(), u32::from_str_radix(t[2], 16).unwrap(), t[3].parse().unwrap(), t[4].parse().unwrap(), t[5].parse().unwrap());
            assert!((0..256).contains(&cur), "{l}");
            assert_eq!(allgreen, 0, "{l}: the all-green byte is not modelled");
            let mut j = cross();
            j.current = cur as u8;
            j.timer = f32::from_bits(timer);
            let state = j.arm_state(arm, 0.0);
            let (idx, want) = match t[0] { "g" => (0, 2), "y" => (1, 1), _ => (2, 0) };
            // red's answer is "the arm is not green and not yellow" = state 0
            assert_eq!(state == want, ret == 1, "{l}: port state {state}");
            n[idx] += 1;
        }
        assert!(n.iter().all(|&k| k >= 10), "{n:?}");
    }

    fn one() -> Signals {
        let nj = NavJunction { index: 0, center: Vec3::ZERO, mid: Vec3::ZERO, arms: vec![Vec3::X, Vec3::Z, -Vec3::X, -Vec3::Z], half: [12.0, 12.0] };
        Signals::build(&[nj], &[]).0
    }

    fn run(s: &mut Signals, seconds: f32, blocked: &[bool], race: Option<(f32, f32)>) {
        for _ in 0..(seconds * 60.0).round() as usize {
            s.step(1.0 / 60.0, blocked, None, None, race);
        }
    }

    /// One arm at a time: 6.5 s green, 2 s yellow, 1.5 s red all round; arm 0 first, 1.5 s in;
    /// with nobody waiting it is arm 0 again.
    #[test]
    fn one_arm_at_a_time_with_the_originals_times() {
        let mut s = one();
        run(&mut s, 1.0, &[], None);
        assert!((0..4).all(|a| s.junctions[0].arm_state(a, 0.0) == 0));
        run(&mut s, 0.6, &[], None);
        assert_eq!(s.junctions[0].arm_state(0, 0.0), 2);
        run(&mut s, 6.5, &[], None);
        assert_eq!(s.junctions[0].arm_state(0, 0.0), 1);
        run(&mut s, 2.0, &[], None);
        assert!((0..4).all(|a| s.junctions[0].arm_state(a, 0.0) == 0));
        run(&mut s, 1.6, &[], None);
        assert_eq!((s.junctions[0].current, s.junctions[0].arm_state(0, 0.0)), (0, 2));
    }

    /// The arm where a car waits is next; the switch waits while a car is in the box; a race's
    /// countdown is red all round.
    #[test]
    fn demand_box_and_countdown() {
        let mut s = one();
        // a car the light refuses at +z, within the reach (12 m): arm 1's demand
        for _ in 0..(11.6f32 * 60.0).round() as usize {
            if s.junctions[0].lane_state(Vec3::Z) != 2 {
                s.wait(0, Vec3::new(0.0, 0.0, 11.0));
            }
            s.step(1.0 / 60.0, &[], None, None, None);
        }
        // (arm 0 had the first green; the car at +z waited through it)
        assert_eq!((s.junctions[0].current, s.junctions[0].arm_state(1, 0.0)), (1, 2));
        // beyond the reach it does not count
        let mut s = one();
        for _ in 0..(11.6f32 * 60.0).round() as usize {
            s.wait(0, Vec3::new(0.0, 0.0, 13.0));
            s.step(1.0 / 60.0, &[], None, None, None);
        }
        assert_eq!(s.junctions[0].current, 0);
        let mut s = one();
        let crossing = [true];
        run(&mut s, 3.0, &crossing, None);
        assert!((0..4).all(|a| s.junctions[0].arm_state(a, 0.0) == 0), "held while the box is occupied");
        run(&mut s, 4.0, &crossing, None);
        assert_eq!(s.junctions[0].arm_state(0, 0.0), 2, "but not beyond 5 s");
        // the player's demand (F-11273): free roam adds one to the arm at its bearing unless
        // that arm is current; in a race above 10 mph a green on another arm is cut short
        let mut s = one();
        s.player_demand(0, Vec3::new(0.0, 0.0, 9.0), false);
        assert_eq!(s.junctions[0].demand, vec![0, 1, 0, 0]);
        s.player_demand(0, Vec3::new(9.0, 0.0, 0.0), false);
        assert_eq!(s.junctions[0].demand, vec![0, 1, 0, 0], "its own arm is current");
        s.junctions[0].timer = 2.0;
        s.player_demand(0, Vec3::new(0.0, 0.0, 9.0), true);
        assert_eq!((s.junctions[0].demand[1], s.junctions[0].timer), (2, 6.5));
        s.junctions[0].timer = 2.0;
        s.player_demand(0, Vec3::new(9.0, 0.0, 0.0), true);
        assert_eq!(s.junctions[0].timer, 0.0, "own green held");
        let mut s = one();
        run(&mut s, 5.0, &[], Some((1.0, 0.0)));
        assert!((0..4).all(|a| s.junctions[0].arm_state(a, 0.0) == 0));
    }

    /// A car is on the arm behind it; a forced arm is held (from the next step), forced red stops all, and so does
    /// the red light race byte, before a forced arm.
    #[test]
    fn approach_arm_and_forcing() {
        let mut j = cross();
        j.timer = 2.0;
        // driving along -x towards the centre = coming in on the +x arm (arm 0)
        assert_eq!(j.state_for(-Vec3::X, 2.0), 2);
        assert_eq!(j.state_for(Vec3::X, 2.0), 0);
        let mut s = one();
        s.junctions[0].forced = 2;
        s.step(1.0 / 60.0, &[], None, None, None);
        let j = &s.junctions[0];
        assert_eq!((j.state_for(Vec3::X, 2.0), j.state_for(-Vec3::X, 2.0)), (2, 0));
        s.red_light_race = true;
        s.step(1.0 / 60.0, &[], None, None, None);
        assert!((0..4).all(|a| s.junctions[0].arm_state(a, 2.0) == 0), "the red light race byte: red all round");
        s.red_light_race = false;
        s.step(1.0 / 60.0, &[], None, None, None);
        assert_eq!(s.junctions[0].arm_state(2, 2.0), 2, "the forced arm again");
        s.junctions[0].forced = -2;
        s.step(1.0 / 60.0, &[], None, None, None);
        assert!((0..4).all(|a| s.junctions[0].arm_state(a, 2.0) == 0));
    }

    /// F-86376 and F-16850 run whole on the original's generated code with the red light race byte set by its native
    /// F-15914 (`tools/red_light_race_reference.py`): the junction update - current arm, timer and demand words - and
    /// the riders' light - go and the demand words - equal for every case, the byte on and off.
    #[test]
    fn red_light_race_matches_the_original() {
        let text = include_str!("testdata/red_light_race_ref.txt");
        let h = |w: &str| u32::from_str_radix(w, 16).unwrap();
        let f = |w: &str| f32::from_bits(h(w));
        let mut n = [0usize; 4];
        for l in text.lines() {
            let (case, out) = l.split_once(" -> ").unwrap();
            let c: Vec<&str> = case.split(' ').collect();
            let o: Vec<u32> = out.split(' ').map(h).collect();
            let k = h(c[1]) as usize;
            let w = &c[1..];
            let arms: Vec<Vec3> = (0..k).map(|a| Vec3::new(f(w[1 + 2 * a]), 0.0, f(w[2 + 2 * a]))).collect();
            let center = Vec3::new(f(w[9]), f(w[10]), f(w[11]));
            let junction = |reach: f32, demand: Vec<u32>| Junction { center, arms: arms.clone(), radius: 12.0, reach, forced: -1, current: 0, timer: 0.0, demand };
            if c[0] == "s" {
                let mut j = junction(12.0, (0..k).map(|a| h(w[21 + a])).collect());
                j.current = h(w[12]) as u8;
                j.timer = f(w[13]);
                j.forced = if h(w[15]) != 0 { -2 } else { h(w[14]) as i32 as i8 };
                let flag = h(w[16]) != 0;
                let clock = h(w[18]) as i32;
                let race = (h(w[17]) != 0).then(|| if clock < 0 { (-clock as f32 / 1000.0, 0.0) } else { (0.0, clock as f32 / 1000.0) });
                let mut s = Signals { junctions: vec![j], red_light_race: flag, ..Default::default() };
                let player = Vec3::new(f(w[25]), f(w[26]), f(w[27]));
                s.step(f(w[20]), &[h(w[19]) != 0], None, Some((player, Vec3::ZERO)), race);
                let j = &s.junctions[0];
                let mut got = vec![j.current as u32, j.timer.to_bits()];
                got.extend((0..4).map(|a| j.demand.get(a).copied().unwrap_or(0)));
                assert_eq!(got, o, "{l}");
                n[flag as usize] += 1;
            } else {
                let mut s = Signals { junctions: vec![junction(f(w[12]), (0..k).map(|a| h(w[17 + a])).collect())], ..Default::default() };
                let flag = h(w[15]) != 0;
                let go = light_go(h(w[13]) != 0, h(w[14]) & 0x20 != 0, flag, || h(w[16]) != 0);
                if !go {
                    s.wait(0, Vec3::new(f(w[21]), f(w[22]), f(w[23])));
                    s.step(0.0, &[], None, None, None);
                }
                let mut got = vec![go as u32];
                got.extend((0..4).map(|a| s.junctions[0].demand.get(a).copied().unwrap_or(0)));
                assert_eq!(got, o, "{l}");
                n[2 + flag as usize] += 1;
            }
        }
        assert!(n.iter().all(|&k| k >= 900), "{n:?}");
    }

    /// The original's lane test (F-78185): the lane's first span (P0 - P1) within 25
    /// degrees of the current arm, green only before 6.5 s, yellow to 8.5 s.
    #[test]
    fn lane_test_reads_the_lanes_first_span() {
        let mut j = cross();
        j.timer = 2.0;
        // a lane entering from arm 0 (+x), its first span 24 and 26 degrees off the arm
        let at = |deg: f32| Vec3::new(deg.to_radians().cos(), 0.0, deg.to_radians().sin());
        assert_eq!(j.lane_state(at(24.0)), 2);
        assert_eq!(j.lane_state(at(26.0)), 0);
        assert_eq!(j.lane_state(Vec3::Z), 0);
        j.timer = 7.0;
        assert_eq!(j.lane_state(at(0.0)), 1);
        j.timer = 8.6;
        assert_eq!(j.lane_state(at(0.0)), 0);
        // (a forced arm: `step` makes it current with the timer at 0)
        j.current = 1;
        j.timer = 0.0;
        assert_eq!((j.lane_state(Vec3::Z), j.lane_state(Vec3::X)), (2, 0));
    }

    /// Heads are taken by the arm they point along; a stray head stays unbound.
    #[test]
    fn heads_bind_to_the_arm_they_point_along() {
        let nj = NavJunction { index: 0, center: Vec3::new(100.0, 5.0, 50.0), mid: Vec3::new(100.0, 0.0, 50.0), arms: vec![Vec3::X, -Vec3::X, Vec3::Z], half: [12.0, 12.0] };
        let heads = [(Vec3::new(108.0, 5.2, 58.0), Vec3::Z), (Vec3::new(92.0, 5.1, 42.0), -Vec3::X), (Vec3::new(108.0, 5.0, 42.0), Vec3::X), (Vec3::new(300.0, 5.0, 50.0), Vec3::X)];
        let (s, of) = Signals::build(&[nj], &heads);
        assert_eq!(s.junctions.len(), 1);
        assert_eq!(of, vec![(0, 2), (0, 1), (0, 0), (UNBOUND, 0)]);
    }
}

#[cfg(test)]
mod original_signal_heads {
    use super::*;

    /// F-83493 run offline as generated over 6000 junction/head/timer states (heads in or out of the list, a missing junction, the
    /// current arm any int, timers around the 6.5 and 8.5 edges, a head without its shader block, the all-green byte): the head's
    /// SignalColor is the port's `arm_light` (a head no junction took shows green, as the port's UNBOUND), the original sets nothing
    /// without a block, and the all-green byte (online only, not modelled) makes it green.
    #[test]
    fn head_colour_matches_the_original() {
        let (mut n, mut seen) = (0, [0usize; 3]);
        for l in include_str!("testdata/map_signal_state_ref.txt").lines() {
            let (i, o) = l.split_once(" -> ").unwrap();
            let f: Vec<&str> = i.split(' ').collect();
            let (jp, idx, cur): (u32, i64, i64) = (f[1].parse().unwrap(), f[3].parse().unwrap(), f[4].parse::<i64>().unwrap());
            let cur = if cur == 0xFFFF_FFFF { -1 } else { cur };
            let timer = f32::from_bits(u32::from_str_radix(f[5], 16).unwrap());
            let (byte, bp, cnt): (u8, u32, u32) = (f[6].parse().unwrap(), f[7].parse().unwrap(), f[8].parse().unwrap());
            let w: Vec<&str> = o.split(' ').collect();
            if bp == 0 || cnt == 0 {
                assert_eq!(w[0], "0", "{l}");
                n += 1;
                continue;
            }
            assert_eq!(w[0], "1", "{l}");
            let rgb: Vec<u32> = w[1..4].iter().map(|x| u32::from_str_radix(x, 16).unwrap()).collect();
            let got = if byte != 0 || jp == 0 || idx < 0 { 2 } else { arm_light(cur, timer, idx) };
            let want = match (rgb[0], rgb[1], rgb[2]) {
                (0x3f80_0000, 0, 0) => 0,
                (0, 0x3f80_0000, 0) => 1,
                (0, 0, 0x3f80_0000) => 2,
                _ => panic!("{l}"),
            };
            assert_eq!(got, want, "{l}");
            if byte == 0 && jp != 0 && idx >= 0 {
                seen[got as usize] += 1;
            }
            n += 1;
        }
        assert_eq!(n, 6000);
        assert!(seen.iter().all(|&k| k > 100), "{seen:?}");
    }

    /// F-77331 (every junction stepped by the frame time) with F-86376 inside it, run offline as generated over 600 cases of 240 frames
    /// (1 .. 6 junctions of 2 .. 4 arms at any phase, frame times from 1/60 to 0.2, the race clock and its event byte, the red light race
    /// flag changing mid-run, a forced arm or all-red set mid-run, a refused car raising a demand word, the box-clear answer changing
    /// every frame): the junctions' current arm, timer and demand words equal the port's `Signals::step` after every frame (a hash of
    /// them per frame) and at the end.
    #[test]
    fn junction_loop_matches_the_original() {
        let h = |w: &str| u32::from_str_radix(w, 16).unwrap();
        let f = |w: &str| f32::from_bits(h(w));
        let (mut n, mut frames_checked) = (0, 0usize);
        for l in include_str!("testdata/map_signals_ref.txt").lines() {
            // (words that look like a code address carry an underscore after four digits)
            let l = &l.replace('_', "");
            let (case, out) = l[2..].split_once(" -> ").unwrap();
            let (hashes, fin) = out.split_once(" |").unwrap();
            let w: Vec<&str> = case.split(' ').collect();
            let (m, frames) = (h(w[0]) as usize, h(w[1]) as usize);
            let dt = f(w[2]);
            let (event, clock0, cstep) = (h(w[3]) != 0, h(w[4]), h(w[5]));
            let (flag0, flagframe, flag1) = (h(w[6]) != 0, h(w[7]), h(w[8]) != 0);
            let (forceframe, forcej, forceval, forcered) = (h(w[9]), h(w[10]) as usize, h(w[11]), h(w[12]) != 0);
            let (waitperiod, waitj, waitarm) = (h(w[13]), h(w[14]) as usize, h(w[15]) as usize);
            let player = Vec3::new(f(w[16]), f(w[17]), f(w[18]));
            let mut s = Signals { red_light_race: flag0, ..Default::default() };
            let mut masks = Vec::new();
            for j in 0..m {
                let q = &w[19 + j * 21..19 + (j + 1) * 21];
                let k = h(q[0]) as usize;
                let arms: Vec<Vec3> = (0..k).map(|a| Vec3::new(f(q[1 + 2 * a]), 0.0, f(q[2 + 2 * a]))).collect();
                s.junctions.push(Junction {
                    center: Vec3::new(f(q[9]), f(q[10]), f(q[11])),
                    arms,
                    radius: 12.0,
                    reach: 12.0,
                    forced: if h(q[15]) != 0 { -2 } else { h(q[14]) as i32 as i8 },
                    current: h(q[12]) as u8,
                    timer: f(q[13]),
                    demand: (0..k).map(|a| h(q[16 + a])).collect(),
                });
                masks.push(h(q[20]));
            }
            let fnv = |s: &Signals| {
                let mut x = 2166136261u32;
                let mut mix = |v: u32| {
                    for b in v.to_le_bytes() {
                        x = (x ^ b as u32).wrapping_mul(16777619);
                    }
                };
                for j in &s.junctions {
                    mix(j.current as u32);
                    mix(j.timer.to_bits());
                    for d in &j.demand {
                        mix(*d);
                    }
                }
                x
            };
            let want: Vec<u32> = hashes.split_whitespace().map(h).collect();
            assert_eq!(want.len(), frames, "{l}");
            for fr in 0..frames {
                if fr as u32 == flagframe {
                    s.red_light_race = flag1;
                }
                if fr as u32 == forceframe {
                    s.junctions[forcej].forced = if forcered { -2 } else { forceval as i32 as i8 };
                }
                if waitperiod > 0 && fr as u32 % waitperiod == 0 && waitarm < s.junctions[waitj].demand.len() {
                    s.junctions[waitj].demand[waitarm] += 1;
                }
                let blocked: Vec<bool> = masks.iter().map(|m| (m >> (fr % 32)) & 1 != 0).collect();
                let clock = clock0.wrapping_add(fr as u32 * cstep) as i32;
                let race = event.then(|| if clock < 0 { (-clock as f32 / 1000.0, 0.0) } else { (0.0, clock as f32 / 1000.0) });
                s.step(dt, &blocked, None, Some((player, Vec3::ZERO)), race);
                assert_eq!(fnv(&s), want[fr], "{l}: frame {fr}");
                frames_checked += 1;
            }
            let mut got = Vec::new();
            for j in &s.junctions {
                got.push(j.current as u32);
                got.push(j.timer.to_bits());
                got.extend((0..4).map(|a| j.demand.get(a).copied().unwrap_or(0)));
            }
            let fin: Vec<u32> = fin.split_whitespace().map(h).collect();
            assert_eq!(got, fin, "{l}");
            n += 1;
        }
        assert_eq!(n, 600);
        assert_eq!(frames_checked, 600 * 240);
    }

    /// The race events' intersection fire F-11036 with its junction lookup F-56988 (the quadtree query F-44023 with mask 4 and
    /// the junctions' box test F-35347) and the arm of the point F-97162, run whole offline on the real city navsys
    /// (tools/junction_force_reference.py -> testdata/junction_force_ref.txt): 6,000 events - centres at, in and about the
    /// junctions' boxes, on their edges to the float and anywhere in the city; points on and between the arms, anywhere, and
    /// zero (+0, -0, NaN, a denormal); green byte 0 / 1 / 0x80; the command-line switch and the manager word now and then.
    /// The lookup's junction must be `NavSys::junction_at`'s and every junction's held state after the fire the original's
    /// (+176 the arm or -1, +180 red all round; the port's `forced`).
    #[test]
    fn junction_force_matches_the_original() {
        let root = std::path::PathBuf::from(std::env::var("MCLA_DATA_DIR").unwrap_or_else(|_| "D:/MCLA/extracted/cache".into()));
        let Ok(res) = crate::assets::load_resource_file(&root.join("resources/city/sc/0x9960fcd8.r66")) else { return };
        let nav = NavSys::parse(&res).unwrap();
        let (mut s, _) = Signals::build(&mcla_formats::rage::navsys::parse_junctions(&res), &[]);
        let h = |t: &str| u32::from_str_radix(t, 16).unwrap();
        let (mut n, mut found, mut wrote) = (0, 0, 0);
        for l in include_str!("testdata/junction_force_ref.txt").lines().filter(|l| !l.is_empty()) {
            let (c, o) = l.split_once(" -> ").unwrap();
            let w: Vec<u32> = c.split(' ').skip(1).map(h).collect();
            let v = |k: usize| Vec3::new(f32::from_bits(w[k]), f32::from_bits(w[k + 1]), f32::from_bits(w[k + 2]));
            let (centre, point) = (v(3), v(6));
            let (q, writes) = o.split_once(" |").unwrap();
            assert_eq!(nav.junction_at(centre).map_or(-1, |j| j as i64), q.parse::<i64>().unwrap(), "lookup: {l}");
            for j in s.junctions.iter_mut() {
                j.forced = 99;
            }
            // (the switch and the manager are the original's guards before anything; the port has neither)
            if w[0] == 0 && w[1] != 0 {
                s.force(Some(&nav), centre, point, w[2] & 0xFF != 0);
            }
            let mut got: Vec<String> = s
                .junctions
                .iter()
                .zip(s.nav_index.iter())
                .filter(|(j, _)| j.forced != 99)
                .map(|(j, &ni)| match j.forced {
                    -2 => format!("{ni} ffffffff 01"),
                    -1 => format!("{ni} ffffffff 00"),
                    a => format!("{ni} {:08x} 00", a),
                })
                .collect();
            if got.is_empty() {
                got.push("-".into());
            }
            assert_eq!(got.join(" "), writes.trim(), "writes: {l}");
            found += (q != "-1") as usize;
            wrote += (writes.trim() != "-") as usize;
            n += 1;
        }
        assert_eq!(n, 6000);
        eprintln!("junction force: {n} events, {found} with a junction, {wrote} holding one");
    }
}
