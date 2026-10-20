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
//! * Each junction keeps its own state (`Signals::step`, the original's update sub_8273ABC0):
//!   the current arm, the phase timer and a demand counter per arm. A car waiting at a red arm
//!   adds to that arm's demand every tick; when the phase is over the arm with the largest
//!   demand is next (arm 0 when nobody waits), and the switch waits - up to 5 s - while a car is
//!   still in the box. In a race every junction is red all round during the countdown, the
//!   first switch after the start goes to the player's arm, and a fast player gets priority at
//!   the junction ahead. The vehicles' copy is stepped; the props' copy takes its state
//!   (`sync_from`).
//!
//! Read only in part from the code, so approximate here: what counts as "waiting" and as "in the
//! box" (see `step`).
//! The lamp state reaches the prop shader through the instance scale (see `Props::update_signals`).

use glam::Vec3;
use hashbrown::HashMap;
use mcla_formats::rage::navsys::NavJunction;

const GREEN: f32 = 6.5;
const YELLOW: f32 = 2.0;
const ALL_RED: f32 = 1.5;
const PHASE: f32 = GREEN + YELLOW + ALL_RED;
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

impl Junction {
    /// The arm a point (or, for `dir`, a direction from the centre) lies on.
    pub fn arm_towards(&self, dir: Vec3) -> usize {
        let d = level(dir);
        (0..self.arms.len()).max_by(|a, b| self.arms[*a].dot(d).total_cmp(&self.arms[*b].dot(d))).unwrap_or(0)
    }

    /// State of an arm: 0 red, 1 yellow, 2 green. (`_t` is not used any more: the state is the
    /// junction's own.)
    pub fn arm_state(&self, arm: usize, _t: f32) -> u8 {
        match self.forced {
            -2 => return 0,
            f if f >= 0 => return if arm == f as usize { 2 } else { 0 },
            _ => {}
        }
        let (current, timer) = (self.current as usize, self.timer);
        if arm != current || timer >= GREEN + YELLOW {
            0
        } else if timer < GREEN {
            2
        } else {
            1
        }
    }

    /// Signal state for traffic travelling along `heading` towards the junction at time `t`:
    /// the car is on the arm that points back the way it came.
    pub fn state_for(&self, heading: Vec3, t: f32) -> u8 {
        self.arm_state(self.arm_towards(-heading), t)
    }
}

#[derive(Default, Clone)]
pub struct Signals {
    pub junctions: Vec<Junction>,
    grid: HashMap<(i32, i32), Vec<u32>>,
    /// junctions whose own lanes carry traffic since the last step (`occupy`)
    held: Vec<u32>,
}

const CELL: f32 = 96.0;

fn cell(p: Vec3) -> (i32, i32) {
    ((p.x / CELL).floor() as i32, (p.z / CELL).floor() as i32)
}

impl Signals {
    /// Hold the lights of the junction at `center`: green for the arm that `dir` (from the
    /// centre: the race scripts give a point on the racers' approach) lies on, or red all
    /// round. Returns false if there is no signalled junction there.
    pub fn force(&mut self, center: Vec3, dir: Vec3, green: bool) -> bool {
        let near = self
            .junctions
            .iter_mut()
            .filter(|j| (j.center.y - center.y).abs() < 9.0)
            .map(|j| (Vec3::new(j.center.x - center.x, 0.0, j.center.z - center.z).length(), j))
            .filter(|e| e.0 < e.1.radius + 22.0)
            .min_by(|a, b| a.0.total_cmp(&b.0));
        match near {
            Some((_, j)) => {
                j.forced = if !green {
                    -2
                } else if dir.length_squared() < 1e-6 {
                    return true;
                } else {
                    j.arm_towards(dir) as i8
                };
                true
            }
            None => false,
        }
    }

    /// The original's reset (sub_8273A4A8): nothing forced, arm 0, red all round for 1.5 s more,
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
    }

    /// One tick of every junction. `cars`: position and velocity of the cars on the road;
    /// `player`: the player's car; `race`: (countdown left, race clock) while there is a race.
    ///
    /// Order as sub_8273ABC0: countdown -> red all round; forced red; forced arm (held green);
    /// else the timer runs, and past 10 s the junction switches once its box is clear or 15 s
    /// are up - in the first 1.5 s of a race to the player's arm, otherwise to the arm with the
    /// largest demand, arm 0 if there is none.
    /// Demand: a car within the junction's reach plus 12 m, slower than 2 m/s, on an arm that is
    /// not green, adds one a tick (the original counts cars its lane logic refuses; the reach
    /// and the speed test are this port's stand-in). Box occupied: a car within the box's reach
    /// less 2 m (the original tests the junction's own lanes).
    /// Player priority (sub_8273A540; in a race, faster than 10 m/s, at the junction ahead):
    /// own arm green -> its green is held; another arm green -> that goes yellow now; yellow /
    /// red all round -> cut short; the player's arm gets demand.
    pub fn step(&mut self, dt: f32, cars: &[(Vec3, Vec3)], player: Option<(Vec3, Vec3)>, race: Option<(f32, f32)>) {
        let n = self.junctions.len();
        let mut occupied = vec![false; n];
        for ji in std::mem::take(&mut self.held) {
            if let Some(o) = occupied.get_mut(ji as usize) {
                *o = true;
            }
        }
        for (pos, vel) in cars {
            let (cx, cz) = cell(*pos);
            for dx in -1..=1 {
                for dz in -1..=1 {
                    let Some(list) = self.grid.get(&(cx + dx, cz + dz)) else { continue };
                    for &ji in list {
                        let j = &mut self.junctions[ji as usize];
                        if (j.center.y - pos.y).abs() > 7.0 {
                            continue;
                        }
                        let off = Vec3::new(pos.x - j.center.x, 0.0, pos.z - j.center.z);
                        let d = off.length();
                        if d < j.radius - 2.0 {
                            occupied[ji as usize] = true;
                        } else if d < j.radius + 12.0 && vel.length() < 2.0 {
                            let arm = j.arm_towards(off);
                            if j.arm_state(arm, 0.0) != 2 {
                                if let Some(c) = j.demand.get_mut(arm) {
                                    *c = c.saturating_add(1);
                                }
                            }
                        }
                    }
                }
            }
        }
        let started = race.map_or(false, |r| r.0 <= 0.0);
        if let (Some((pos, vel)), true) = (player, started) {
            if vel.length() > 10.0 {
                if let Some((ji, _, _)) = self.ahead(pos, vel, 0.0) {
                    let j = &mut self.junctions[ji];
                    if j.forced == -1 {
                        let arm = j.arm_towards(pos - j.center);
                        if j.timer < GREEN && j.current as usize == arm {
                            j.timer = 0.0;
                        } else {
                            if let Some(c) = j.demand.get_mut(arm) {
                                *c = c.saturating_add(1);
                            }
                            j.timer = if j.timer < GREEN { GREEN } else if j.timer < GREEN + YELLOW { j.timer.max(8.25) } else { j.timer.max(9.75) };
                        }
                    }
                }
            }
        }
        for (ji, j) in self.junctions.iter_mut().enumerate() {
            if race.map_or(false, |r| r.0 > 0.0) || j.forced == -2 {
                j.timer = PHASE;
                continue;
            }
            if j.forced >= 0 {
                j.current = j.forced as u8;
                j.timer = 0.0;
                continue;
            }
            j.timer += dt;
            if j.timer <= PHASE || (occupied[ji] && j.timer <= PHASE + 5.0) {
                continue;
            }
            j.timer = 0.0;
            let next = match (race, player) {
                (Some((_, clock)), Some((pos, _))) if clock < 1.5 => j.arm_towards(pos - j.center),
                _ => (0..j.demand.len()).filter(|a| j.demand[*a] > 0).max_by_key(|a| j.demand[*a]).unwrap_or(0),
            };
            j.current = next as u8;
            if let Some(c) = j.demand.get_mut(next) {
                *c = 0;
            }
        }
    }

    /// The road network's junctions, and for every signal head (position, z axis) the junction
    /// and arm it is bound to (`UNBOUND`: none). Binding as the original's `sub_8273AF48`: the
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
            let arms: Vec<Vec3> = nj.arms.iter().map(|a| level(*a)).collect();
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
            out.push(Junction { center: nj.center, arms, radius: nj.half[0].max(nj.half[1]).clamp(6.0, 40.0), forced: -1, current: 0, timer: START, demand });
        }
        let mut grid: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
        for (i, j) in out.iter().enumerate() {
            grid.entry(cell(j.center)).or_default().push(i as u32);
        }
        (Signals { junctions: out, grid, held: Vec::new() }, of_head)
    }

    /// A traffic car on one of a junction's own lanes (its through lanes and connectors): the
    /// junction near `pos` counts as occupied at its next step - the original holds the phase
    /// while the junction's lanes carry traffic.
    pub fn occupy(&mut self, pos: Vec3) {
        let (cx, cz) = cell(pos);
        let mut best: Option<(u32, f32)> = None;
        for dx in -1..=1 {
            for dz in -1..=1 {
                let Some(list) = self.grid.get(&(cx + dx, cz + dz)) else { continue };
                for &ji in list {
                    let j = &self.junctions[ji as usize];
                    let d = Vec3::new(pos.x - j.center.x, 0.0, pos.z - j.center.z).length();
                    if (j.center.y - pos.y).abs() < 7.0 && d < j.radius + 12.0 && best.map_or(true, |b| d < b.1) {
                        best = Some((ji, d));
                    }
                }
            }
        }
        if let Some((ji, _)) = best {
            if !self.held.contains(&ji) {
                self.held.push(ji);
            }
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
        Junction { center: Vec3::ZERO, arms: vec![Vec3::X, Vec3::Z, -Vec3::X, -Vec3::Z], radius: 12.0, forced: -1, current: 0, timer: START, demand: vec![0; 4] }
    }

    fn one() -> Signals {
        let nj = NavJunction { center: Vec3::ZERO, arms: vec![Vec3::X, Vec3::Z, -Vec3::X, -Vec3::Z], half: [12.0, 12.0] };
        Signals::build(&[nj], &[]).0
    }

    fn run(s: &mut Signals, seconds: f32, cars: &[(Vec3, Vec3)], race: Option<(f32, f32)>) {
        for _ in 0..(seconds * 60.0).round() as usize {
            s.step(1.0 / 60.0, cars, None, race);
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
        let waiting = [(Vec3::new(0.0, 0.0, 18.0), Vec3::ZERO)];
        run(&mut s, 11.6, &waiting, None);
        // (arm 0 had the first green; the car at +z waited through it)
        assert_eq!((s.junctions[0].current, s.junctions[0].arm_state(1, 0.0)), (1, 2));
        let mut s = one();
        let crossing = [(Vec3::new(1.0, 0.0, 1.0), Vec3::new(5.0, 0.0, 0.0))];
        run(&mut s, 3.0, &crossing, None);
        assert!((0..4).all(|a| s.junctions[0].arm_state(a, 0.0) == 0), "held while the box is occupied");
        run(&mut s, 4.0, &crossing, None);
        assert_eq!(s.junctions[0].arm_state(0, 0.0), 2, "but not beyond 5 s");
        let mut s = one();
        run(&mut s, 5.0, &[], Some((1.0, 0.0)));
        assert!((0..4).all(|a| s.junctions[0].arm_state(a, 0.0) == 0));
    }

    /// A car is on the arm behind it; a forced arm is held, forced red stops all.
    #[test]
    fn approach_arm_and_forcing() {
        let mut j = cross();
        j.timer = 2.0;
        // driving along -x towards the centre = coming in on the +x arm (arm 0)
        assert_eq!(j.state_for(-Vec3::X, 2.0), 2);
        assert_eq!(j.state_for(Vec3::X, 2.0), 0);
        j.forced = 2;
        assert_eq!((j.state_for(Vec3::X, 2.0), j.state_for(-Vec3::X, 2.0)), (2, 0));
        j.forced = -2;
        assert!((0..4).all(|a| j.arm_state(a, 2.0) == 0));
    }

    /// Heads are taken by the arm they point along; a stray head stays unbound.
    #[test]
    fn heads_bind_to_the_arm_they_point_along() {
        let nj = NavJunction { center: Vec3::new(100.0, 5.0, 50.0), arms: vec![Vec3::X, -Vec3::X, Vec3::Z], half: [12.0, 12.0] };
        let heads = [(Vec3::new(108.0, 5.2, 58.0), Vec3::Z), (Vec3::new(92.0, 5.1, 42.0), -Vec3::X), (Vec3::new(108.0, 5.0, 42.0), Vec3::X), (Vec3::new(300.0, 5.0, 50.0), Vec3::X)];
        let (s, of) = Signals::build(&[nj], &heads);
        assert_eq!(s.junctions.len(), 1);
        assert_eq!(of, vec![(0, 2), (0, 1), (0, 0), (UNBOUND, 0)]);
    }
}
