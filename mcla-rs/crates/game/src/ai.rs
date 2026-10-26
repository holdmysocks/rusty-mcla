//! AI driver: follows the directed road-link graph with a look-ahead pursuit controller.

use crate::physics::{CarInput, CarState};
use glam::Vec3;
use mcla_formats::rage::navsys::NavSys;

/// A computer driver's pre-race antics on the grid (`AiDriver::grid_input`): the phase it is in, when
/// (seconds since placement) that phase ends, when (seconds to GO) it commits to the burnout it holds
/// to GO, and the throttle lifts during that burnout as (from, to) in seconds to GO.
#[derive(Debug, Clone, Default)]
pub struct GridPlan {
    pub t: f32,
    pub phase: GridPhase,
    pub until: f32,
    pub commit_at: f32,
    pub lifts: Vec<(f32, f32)>,
    pub committed: bool,
    /// set when the opening burst ends: the car's drivetrain is reset (vehsim.rs `grid_reset`)
    pub reset: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum GridPhase {
    /// the pedals off but the brake
    #[default]
    Off,
    /// the burst right after placement: handbrake and throttle for a quarter second
    Burst,
    /// revving against the brake
    Rev,
    /// a burnout: handbrake and throttle
    Burn,
    /// the handbrake and the brake (the charge decaying)
    HandOnly,
}

#[derive(Debug, Clone, Default)]
pub struct AiDriver {
    pub link: Option<usize>,
    pub t: f32,
    /// planned route: links after the current one
    pub route: Vec<usize>,
    pub target_speed: f32, // m/s
    pub aggression: f32,   // 0..1
    pub stuck_timer: f32,
    pub reverse_timer: f32,
    pub rng: u32,
    pub last_target: Vec3,
    /// fixed route (race): links to follow in order
    pub route_override: Option<Vec<usize>>,
    /// cursor into `route_override` (links may repeat across laps)
    pub route_pos: usize,
    /// race line: polyline to follow (takes precedence over the link route)
    pub path: Option<std::sync::Arc<Vec<Vec3>>>,
    /// index of the path segment we are on (segment i = path[i] -> path[i + 1])
    pub path_i: usize,
    /// time spent (nearly) stationary while trying to drive
    pub stuck_long: f32,
    /// position the car has to get 6 m away from to count as making progress
    pub stuck_anchor: Vec3,
    /// set when the car should be put back on the race line: (position, yaw)
    pub reset_to: Option<(Vec3, f32)>,
    /// racing: how far beside the line to drive just now (metres, + = right) to get past traffic
    pub dodge: f32,
    /// pursuit target (position, velocity): police cruisers
    pub chase: Option<(Vec3, Vec3)>,
    /// the car as its driver knows it: the game's calibration (speed for a bend, braking, steering)
    pub tune: Option<std::sync::Arc<mcla_formats::rage::aitune::AiTune>>,
    /// the pre-race antics on the grid (`grid_input`)
    pub grid: Option<GridPlan>,
    /// the speed asked for already allows for the bends ahead (set while following a race line)
    line_speed: bool,
    /// racing: seconds boxed in behind something standing
    pub blocked: f32,
    /// diagnostics: the speed last aimed for
    pub last_v_target: f32,
    pub chase_replan: f32,
    /// forward phase of a three-point turn: no new reverse before this runs out
    pub kturn_cool: f32,
    /// racing: over the finish line - on the brake, still steering along the line (the original's
    /// racers stop 60 - 65 m past the line, recorded 2026-10-19: CAREER_RACES.md "Race end")
    pub finished: bool,
    /// racing: the rubber band's state for this racer (game.rs, mcRubberBandMgr)
    pub rb: Option<RbState>,
    /// racing: the original's racer brain drives this car (brain.rs); the rest of this driver is unused
    pub brain: crate::brain::BrainSlot,
    /// racing: another car ran into this one (the contact walker's notify, `Vehicles::notify_contact`); the brain takes it as its
    /// 1.5 s collision timer at its next update
    pub hit_pending: bool,
    /// racing: the race's checkpoints and alt paths for the brain, and this racer's index in the race
    /// (the bit of the alt paths' opponent masks; the player is 0)
    pub course: Option<std::sync::Arc<crate::brain::Course>>,
    pub opponent: u32,
}

/// What the rubber band measures for a racer each frame (brain +384 .. +404, docs/AI.md "Rubber banding").
#[derive(Debug, Clone, Copy, Default)]
pub struct RbState {
    /// where it started (racer +192)
    pub start: Vec3,
    /// metres ahead of the best-placed human along the checkpoints (+388; behind < 0)
    pub gap: f32,
    /// along the checkpoints from its start (+392)
    pub from_start: f32,
    /// to the finish (+396)
    pub to_finish: f32,
    /// the throttle cap (+404)
    pub cap: f32,
}

impl AiDriver {
    pub fn new(seed: u32) -> AiDriver {
        AiDriver { target_speed: 20.0, aggression: 0.5, rng: seed.max(1), ..Default::default() }
    }

    fn rand(&mut self) -> f32 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        (x % 10000) as f32 / 10000.0
    }

    /// a draw between `lo` and `hi` seconds
    fn span(&mut self, lo: f32, hi: f32) -> f32 {
        lo + self.rand() * (hi - lo)
    }

    /// What a computer driver does on the grid while the race has not started (N-VP-1), with `to_go`
    /// seconds to GO. Recorded on the original 2026-10-18 (`work/physics2/rec`: a.1, c2, c3, c4 - an R8,
    /// a Murcielago and a Ducati 999 over four starts of AROUND HOLLYWOOD):
    /// - about 0.1 - 0.2 s after placement every car does a burst of handbrake and full throttle for
    ///   0.2 - 0.3 s (the charge reaches 0.2 - 0.5), then lets everything go for 0.5 - 1.4 s;
    /// - then, in no fixed order, phases of revving against the brake (brake and throttle 1 - the rev
    ///   limiter cuts the pedal from a half, so the engine saws between its limits; 0.8 - 2.6 s), of a
    ///   burnout (handbrake and throttle 1, the charge to full and held; 0.9 - 2.7 s, after which the
    ///   handbrake stays on alone while the charge decays and for 0.7 - 1.3 s more), and of nothing but
    ///   the brake (0.7 - 1.3 s);
    /// - 3.5 - 3.9 s before GO the car commits to a burnout held to GO, lifting the throttle (handbrake
    ///   kept) once or twice for 0.15 - 0.7 s; at GO the handbrake goes with the throttle down and the
    ///   charge launches the car (the burnout's boost, `vehsim.rs`).
    /// The phase kinds and lengths are drawn from those ranges; the original's brain randomness itself
    /// is not decoded (approximate). The sim does the rest: the rev limiter's cut, the burnout's axle,
    /// its charge, the launch. The inputs carry `hold` (the grid flag): the pedals reach the chassis as
    /// they are, the gearbox is held whenever the handbrake is off (so revving against the brake stands
    /// still) and the brake never selects reverse. As recorded, the brain's brake is on in every phase
    /// but a burnout: alone when idle, with the throttle when revving, with the handbrake while a charge
    /// decays.
    pub fn grid_input(&mut self, to_go: f32, dt: f32, _bike: bool) -> CarInput {
        if self.grid.is_none() {
            let commit_at = self.span(3.5, 3.9);
            let n = (self.rand() * 3.0) as usize;
            let mut lifts = Vec::new();
            for _ in 0..n {
                let len = self.span(0.15, 0.7);
                let from = self.span(1.0, (commit_at - 0.5).max(1.0));
                lifts.push((from, (from - len).max(0.25)));
            }
            let first = self.span(0.1, 0.2);
            self.grid = Some(GridPlan { t: 0.0, phase: GridPhase::Off, until: first, commit_at, lifts, committed: false, reset: false });
        }
        let mut g = self.grid.take().unwrap();
        g.t += dt;
        if to_go <= g.commit_at && !g.committed {
            g.committed = true;
            g.phase = GridPhase::Burn;
            g.until = f32::INFINITY;
        }
        if !g.committed && g.t >= g.until {
            // the next phase: after the burst and its release the kinds alternate at random
            let (phase, len) = match g.phase {
                GridPhase::Off if g.t < 1.0 && g.until < 0.5 => (GridPhase::Burst, self.span(0.2, 0.32)),
                GridPhase::Burst => {
                    // (recorded: the burst ends with the drivetrain reset in place, a.1 k115)
                    g.reset = true;
                    (GridPhase::Off, self.span(0.5, 1.4))
                }
                GridPhase::Rev => (GridPhase::Off, self.span(0.7, 1.3)),
                // (the handbrake stays on while the charge decays - 0.85 s from full - and a little longer)
                GridPhase::Burn => (GridPhase::HandOnly, self.span(0.9, 2.0)),
                GridPhase::Off | GridPhase::HandOnly => {
                    if self.rand() < 0.5 { (GridPhase::Rev, self.span(0.8, 2.6)) } else { (GridPhase::Burn, self.span(0.9, 2.7)) }
                }
            };
            g.phase = phase;
            g.until = g.t + len;
        }
        let mut phase = g.phase;
        if g.committed && g.lifts.iter().any(|&(from, to)| to_go <= from && to_go > to) {
            phase = GridPhase::HandOnly;
        }
        self.grid = Some(g);
        let (ai, hold) = (true, true);
        match phase {
            GridPhase::Burst | GridPhase::Burn => CarInput { throttle: 1.0, handbrake: true, ai, hold, ..Default::default() },
            GridPhase::Rev => CarInput { throttle: 1.0, brake: 1.0, ai, hold, ..Default::default() },
            GridPhase::HandOnly => CarInput { handbrake: true, brake: 1.0, ai, hold, ..Default::default() },
            GridPhase::Off => CarInput { brake: 1.0, ai, hold, ..Default::default() },
        }
    }

    pub fn locate(&mut self, nav: &NavSys, pos: Vec3, heading: Vec3) {
        if let Some((li, t)) = nav.nearest_link_aligned(pos, heading) {
            self.link = Some(li);
            self.t = t;
            self.route.clear();
        }
    }

    /// Extend the planned route so that it covers at least `dist` metres ahead.
    fn plan(&mut self, nav: &NavSys, dist: f32) {
        let Some(cur) = self.link else { return };
        if let Some(ro) = self.route_override.as_ref() {
            let start = self.route_pos.min(ro.len());
            let win_end = (start + 80).min(ro.len());
            if let Some(off) = ro[start..win_end].iter().position(|&l| l == cur) {
                let pos = start + off;
                self.route_pos = pos;
                self.route = ro[pos + 1..(pos + 1 + 60).min(ro.len())].to_vec();
                return;
            }
            // off the route: rejoin at the nearest upcoming route link
            let p = nav.links[cur].a;
            let mut best = None;
            let mut bd = f32::INFINITY;
            for (i, &l) in ro.iter().enumerate().take(win_end).skip(start) {
                let d = nav.links[l].a.distance(p);
                if d < bd {
                    bd = d;
                    best = Some(i);
                }
            }
            if let Some(i) = best {
                self.route = ro[i..(i + 60).min(ro.len())].to_vec();
                return;
            }
        }
        let mut acc = nav.links[cur].len * (1.0 - self.t);
        let mut last = cur;
        for &l in &self.route {
            acc += nav.links[l].len;
            last = l;
        }
        let mut guard = 0;
        while acc < dist && guard < 32 {
            guard += 1;
            let l = &nav.links[last];
            if l.next.is_empty() {
                break;
            }
            // prefer going straight, with some randomness
            let d = l.dir();
            let mut best = None;
            let mut best_score = f32::NEG_INFINITY;
            for &n in &l.next {
                if self.route.contains(&n) || n == cur {
                    continue;
                }
                let s = nav.links[n].dir().dot(d) + self.rand() * 0.9;
                if s > best_score {
                    best_score = s;
                    best = Some(n);
                }
            }
            let Some(n) = best else { break };
            acc += nav.links[n].len;
            self.route.push(n);
            last = n;
        }
    }

    fn advance(&mut self, nav: &NavSys, pos: Vec3) {
        let Some(cur) = self.link else { return };
        let l = &nav.links[cur];
        self.t = l.project(pos);
        // move to the next link when we pass the end of the current one
        if self.t >= 0.999 || (pos - l.b).dot(l.dir()) > 0.5 {
            if !self.route.is_empty() {
                let n = self.route.remove(0);
                self.link = Some(n);
                self.t = nav.links[n].project(pos);
            }
        }
    }

    /// Point `dist` metres ahead along current link + route.
    fn point_ahead(&self, nav: &NavSys, dist: f32) -> Option<Vec3> {
        let cur = self.link?;
        let l = &nav.links[cur];
        let mut remaining = dist;
        let left = l.len * (1.0 - self.t);
        if left >= remaining {
            return Some(l.point_at(self.t + remaining / l.len.max(0.01)));
        }
        remaining -= left;
        for &r in &self.route {
            let rl = &nav.links[r];
            if rl.len >= remaining {
                return Some(rl.point_at(remaining / rl.len.max(0.01)));
            }
            remaining -= rl.len;
        }
        self.route.last().map(|&r| nav.links[r].b).or(Some(l.b))
    }

    /// Point `dist` metres ahead of `pos` along the race polyline (advances the cursor).
    /// The fastest the car may go now so that every bend of the line ahead can still be
    /// reached at the speed the car's tables give for it (braking at a share of its limit).
    fn path_speed_limit(&self, pos: Vec3, speed: f32) -> Option<f32> {
        let t = self.tune.as_ref()?;
        let path = self.path.as_ref()?;
        if path.len() < 4 {
            return None;
        }
        let brake = t.max_brake_decel * 0.7;
        let reach = speed * speed / (2.0 * brake) + 40.0;
        // sample the line every few metres; a bend is the circle through three samples
        let step = 7.0f32;
        let mut pts: Vec<Vec3> = vec![pos];
        let mut k = self.path_i.min(path.len() - 2);
        let mut cur = {
            let (a, b) = (path[k], path[k + 1]);
            let ab = b - a;
            a + ab * ((pos - a).dot(ab) / ab.length_squared().max(0.01)).clamp(0.0, 1.0)
        };
        let mut left = step;
        let mut total = 0.0;
        while total < reach && k + 1 < path.len() {
            let to_next = path[k + 1] - cur;
            let l = to_next.length();
            if l < left {
                left -= l;
                total += l;
                cur = path[k + 1];
                k += 1;
            } else {
                cur += to_next / l.max(1e-4) * left;
                total += left;
                left = step;
                pts.push(cur);
            }
        }
        if pts.len() < 3 {
            return None;
        }
        let mut limit = f32::INFINITY;
        let mut d = 0.0;
        for w in pts.windows(3) {
            d += w[0].distance(w[1]);
            let (a, b, c) = (w[0], w[1], w[2]);
            let (ab, bc, ca) = (Vec3::new(b.x - a.x, 0.0, b.z - a.z), Vec3::new(c.x - b.x, 0.0, c.z - b.z), Vec3::new(a.x - c.x, 0.0, a.z - c.z));
            let area2 = ab.cross(bc).length();
            let den = ab.length() * bc.length() * ca.length();
            let kappa = if den > 1e-3 { 2.0 * area2 / den } else { 0.0 };
            let v_bend = t.speed_for_curvature(kappa) * 0.9;
            // (the first sample is the car itself: its own place is not a bend of the line)
            let dd = (d - step * 0.5).max(0.0);
            limit = limit.min((v_bend * v_bend + 2.0 * brake * dd).sqrt());
        }
        Some(limit)
    }

    /// Put `path_i` on the segment of the path nearest to `pos` from `path_i` on: a car the racer brain drives
    /// never moves the index, so it can stand far behind the car (`path_ahead` then led the car back to it -
    /// the Corvette set back 144 m on hw_ordered_jp_04 that turned round in a gas station, AI.md NEW 5).
    pub fn seat_path(&mut self, pos: Vec3) {
        let Some(path) = self.path.as_ref() else { return };
        let mut best = (f32::MAX, self.path_i);
        for i in self.path_i..path.len().saturating_sub(1) {
            let (a, b) = (path[i], path[i + 1]);
            let ab = Vec3::new(b.x - a.x, 0.0, b.z - a.z);
            let t = (Vec3::new(pos.x - a.x, 0.0, pos.z - a.z).dot(ab) / ab.length_squared().max(0.01)).clamp(0.0, 1.0);
            let d = Vec3::new(pos.x - a.x - ab.x * t, 0.0, pos.z - a.z - ab.z * t).length_squared();
            if d < best.0 {
                best = (d, i);
            }
        }
        self.path_i = best.1;
    }

    pub fn path_ahead(&mut self, pos: Vec3, dist: f32) -> Option<Vec3> {
        let path = self.path.clone()?;
        if path.len() < 2 {
            return path.first().copied();
        }
        // advance while we are past the end of the current segment (or closer to a later one)
        let mut guard = 0;
        while self.path_i + 2 < path.len() && guard < 64 {
            guard += 1;
            let a = path[self.path_i];
            let b = path[self.path_i + 1];
            let ab = Vec3::new(b.x - a.x, 0.0, b.z - a.z);
            let l2 = ab.length_squared().max(0.01);
            let t = Vec3::new(pos.x - a.x, 0.0, pos.z - a.z).dot(ab) / l2;
            let near_end = Vec3::new(pos.x - b.x, 0.0, pos.z - b.z).length() < 9.0;
            if t >= 1.0 || near_end {
                self.path_i += 1;
            } else {
                break;
            }
        }
        let i = self.path_i.min(path.len() - 2);
        let a = path[i];
        let b = path[i + 1];
        let ab = b - a;
        let len = ab.length().max(0.01);
        let t = ((pos - a).dot(ab) / (len * len)).clamp(0.0, 1.0);
        let mut remaining = dist;
        let left = len * (1.0 - t);
        if left >= remaining {
            return Some(a + ab * (t + remaining / len));
        }
        remaining -= left;
        let mut k = i + 1;
        while k + 1 < path.len() {
            let seg = path[k + 1] - path[k];
            let sl = seg.length().max(0.01);
            if sl >= remaining {
                return Some(path[k] + seg * (remaining / sl));
            }
            remaining -= sl;
            k += 1;
        }
        path.last().copied()
    }

    pub fn drive(&mut self, nav: &NavSys, car: &CarState, dt: f32) -> CarInput {
        let pos = car.pos;
        let speed = car.fwd_speed();
        let look = (7.0 + speed * 0.9).clamp(7.0, 45.0);
        if let Some((tp, tv)) = self.chase {
            let dist = tp.distance(pos);
            self.chase_replan -= dt;
            let mut direct = dist < 55.0;
            if !direct && (self.chase_replan <= 0.0 || self.path.is_none()) {
                self.chase_replan = 1.5;
                match nav.route_points(pos, Some(car.forward()), tp) {
                    Some(pts) if pts.len() >= 2 => {
                        self.path = Some(std::sync::Arc::new(pts));
                        self.path_i = 0;
                    }
                    _ => {
                        self.path = None;
                        direct = true;
                    }
                }
            }
            if direct {
                self.path = None;
                self.stuck_long = 0.0;
                if dist < 6.5 && tv.length() < 3.0 {
                    // boxed the target in (or told to hold): stop
                    return CarInput { brake: 1.0, handbrake: speed.abs() < 2.0, ..Default::default() };
                }
                let lead = (dist / speed.abs().max(12.0)).min(1.2);
                let target = tp + tv * lead;
                let save = self.target_speed;
                if dist < 16.0 {
                    self.target_speed = (tv.length() + 4.0).min(save);
                }
                let inp = self.steer_towards(car, target, target, dt);
                self.target_speed = save;
                return inp;
            }
        }
        if self.path.is_some() {
            // hopelessly stuck (wedged against scenery): go back onto the race line,
            // like the original's reset-to-track
            if pos.distance(self.stuck_anchor) > 6.0 {
                self.stuck_anchor = pos;
                self.stuck_long = 0.0;
            } else {
                self.stuck_long += dt;
            }
            if self.stuck_long > 7.0 {
                self.stuck_long = 0.0;
                if let Some(path) = self.path.clone() {
                    let i = (self.path_i + 1).min(path.len().saturating_sub(2));
                    let a = path[i];
                    let b = path[(i + 1).min(path.len() - 1)];
                    let d = (b - a).normalize_or(car.forward());
                    self.path_i = i;
                    if path.len() > 20 && i + 3 < path.len() {
                        log::info!("ai: stuck at {:.0} {:.0} {:.0} (line point {} of {}), speed {:.1}", pos.x, pos.y, pos.z, i, path.len(), speed);
                    }
                    self.reset_to = Some((a + Vec3::Y * 0.9, (-d.x).atan2(-d.z)));
                    self.stuck_timer = 0.0;
                    self.reverse_timer = 0.0;
                }
            }
            // (aiming far ahead cuts the corners - into the lamp post on the kerb; the far
            // point only sets the speed)
            let near = (5.0 + speed * 0.5).clamp(6.0, 24.0);
            let Some(target) = self.path_ahead(pos, near) else { return CarInput { brake: 1.0, ..Default::default() } };
            let far = self.path_ahead(pos, look * 2.5).unwrap_or(target);
            // (pulled out to pass: aim beside the line)
            let side = car.right() * self.dodge;
            // (with the car's tables the whole line ahead sets the speed)
            let save = self.target_speed;
            if let Some(v) = self.path_speed_limit(pos, speed) {
                self.target_speed = save.min(v.max(7.0));
                self.line_speed = true;
            }
            let inp = self.steer_towards(car, target + side, far + side * 0.5, dt);
            self.target_speed = save;
            self.line_speed = false;
            return inp;
        }
        if self.link.is_none() {
            self.locate(nav, pos, car.forward());
        }
        self.advance(nav, pos);
        self.plan(nav, 120.0);
        // off the road? relocate
        if let Some(cur) = self.link {
            let l = &nav.links[cur];
            if l.point_at(self.t).distance(pos) > 25.0 {
                if let Some(ro) = self.route_override.clone() {
                    // snap to nearest route link
                    let mut best = None;
                    let mut bd = f32::INFINITY;
                    let start = self.route_pos.min(ro.len());
                    let end = (start + 80).min(ro.len());
                    let mut best_i = start;
                    for (i, &li) in ro.iter().enumerate().take(end).skip(start) {
                        let t = nav.links[li].project(pos);
                        let d = nav.links[li].point_at(t).distance(pos);
                        if d < bd {
                            bd = d;
                            best = Some((li, t));
                            best_i = i;
                        }
                    }
                    if let Some((li, t)) = best {
                        self.link = Some(li);
                        self.t = t;
                        self.route_pos = best_i;
                        self.route.clear();
                    }
                } else {
                    self.locate(nav, pos, car.forward());
                }
                self.plan(nav, 120.0);
            }
        }
        let Some(target) = self.point_ahead(nav, look) else {
            self.link = None;
            return CarInput { throttle: 0.3, ..Default::default() };
        };
        let far = self.point_ahead(nav, look * 2.5).unwrap_or(target);
        self.steer_towards(car, target, far, dt)
    }

    /// Steering / throttle towards `target`, slowing for the curvature seen at `far`.
    fn steer_towards(&mut self, car: &CarState, target: Vec3, far: Vec3, dt: f32) -> CarInput {
        let pos = car.pos;
        let speed = car.fwd_speed();
        self.last_target = target;
        let to = target - pos;
        let fwd = car.forward();
        let right = car.right();
        let dist = to.length().max(0.1);
        let angle = (right.dot(to) / dist).clamp(-1.0, 1.0).asin();
        let steer = (angle * 2.5).clamp(-1.0, 1.0);
        // slow down for upcoming curvature
        let to_far = far - pos;
        let curve = 1.0 - fwd.dot(to_far.normalize_or(fwd)).clamp(0.0, 1.0);
        let mut v_target = (self.target_speed * (1.0 - curve * 1.2 - angle.abs() * 0.8)).max(7.0);
        let mut input = CarInput::default();
        if car.veh.is_some() {
            // on the original's simulation the tyres hold about 2 g and the brakes 1.5:
            // the bend ahead (an arc through the far point) sets the speed it can be taken
            // at, the distance to it how much faster the car may still go
            let d = to_far.length().max(1.0);
            let sin = (1.0 - fwd.dot(to_far / d).clamp(-1.0, 1.0).powi(2)).max(0.0).sqrt();
            let kappa = (2.0 * sin / d).max(1e-4);
            // (with the car's own tables: the speed it holds that bend at, and its braking;
            // a margin is kept, as the tables are the car's limit)
            let (v_bend, brake) = match self.tune.as_ref() {
                Some(t) => (t.speed_for_curvature(kappa) * 0.88, t.max_brake_decel * 0.7),
                None => ((15.0f32 / kappa).sqrt(), 11.0),
            };
            let v_now = if self.line_speed { f32::INFINITY } else { (v_bend * v_bend + 2.0 * brake * d * 0.5).sqrt() };
            v_target = self.target_speed.min(v_now).max(7.0);
            if fwd.dot(to_far) < 0.0 {
                v_target = v_target.min(9.0);
            }
            if speed < v_target {
                input.throttle = ((v_target - speed) * 0.5).clamp(0.3, 1.0);
            } else {
                // a little too fast: lift (the engine brakes hard); more: brake
                input.brake = ((speed - v_target - 1.5) * 0.25).clamp(0.0, 1.0);
            }
        } else if speed < v_target {
            input.throttle = ((v_target - speed) * 0.5).clamp(0.25, 1.0);
        } else {
            input.brake = ((speed - v_target) * 0.35).clamp(0.0, 1.0);
        }
        self.last_v_target = v_target;
        input.steer = steer;
        if let (Some(t), true, true) = (self.tune.as_ref(), car.veh.is_some(), std::env::var("MCLA_AI_STEER_OLD").is_err()) {
            // the steering the tables give for the arc to the target (curvature x SteerScalar at
            // this speed), a little over to pull onto the line
            let k = 2.0 * right.dot(to) / (dist * dist);
            input.steer = (k * t.steer_scalar_at(speed) * 1.25).clamp(-1.0, 1.0);
        }
        self.kturn_cool = (self.kturn_cool - dt).max(0.0);
        if fwd.dot(to) < -0.2 * dist {
            input.throttle = if self.chase.is_some() { 0.8 } else { 0.5 };
            input.brake = 0.0;
            input.steer = if right.dot(to) > 0.0 { 1.0 } else { -1.0 };
            // three-point turn: back up with opposite lock, then go forward again
            if self.chase.is_some() && speed.abs() < 3.0 && self.reverse_timer <= 0.0 && self.kturn_cool <= 0.0 {
                self.reverse_timer = 1.5;
                self.kturn_cool = 4.0;
            }
        }
        if speed.abs() < 0.5 && input.throttle > 0.3 {
            self.stuck_timer += dt;
        } else {
            self.stuck_timer = 0.0;
        }
        if self.stuck_timer > 1.5 {
            self.reverse_timer = 1.2;
            self.stuck_timer = 0.0;
        }
        // (the rubber band caps the throttle the driver asks for: sub_822BCFE0 0x822BD544..0x822BD668)
        if let Some(r) = self.rb {
            input.throttle = input.throttle.min(r.cap);
        }
        if self.reverse_timer > 0.0 {
            self.reverse_timer -= dt;
            input.throttle = 0.0;
            input.brake = 1.0;
            input.steer = -steer;
        }
        if self.finished {
            input.throttle = 0.0;
            input.brake = 1.0;
        }
        input
    }
}

#[cfg(test)]
mod grid_tests {
    use super::*;

    /// The pre-race antics (N-VP-1) keep the shape of the original's recordings: a burst of handbrake
    /// and throttle inside the first 0.6 s, the brake on in every phase but a burnout, the grid flag on
    /// every input, a burnout committed 3.5 - 3.9 s before GO and held through the last quarter second.
    #[test]
    fn grid_antics_have_the_recorded_shape() {
        for seed in [1u32, 77, 1234, 99991] {
            let mut d = AiDriver::new(seed);
            let dt = 1.0 / 60.0;
            let mut burst_at = None;
            let mut committed_at = None;
            let mut t = 0.0f32;
            while t < 12.0 {
                let to_go = 12.0 - t;
                let i = d.grid_input(to_go, dt, false);
                assert!(i.ai && i.hold);
                assert!(!(i.handbrake && i.throttle > 0.0 && i.brake > 0.0), "brake in a burnout at t {t}");
                assert!(i.handbrake || i.brake > 0.0, "nothing holds the car at t {t}");
                if i.handbrake && i.throttle > 0.0 && burst_at.is_none() {
                    burst_at = Some(t);
                }
                if d.grid.as_ref().unwrap().committed && committed_at.is_none() {
                    committed_at = Some(to_go);
                }
                if to_go <= 0.25 {
                    assert!(i.handbrake && i.throttle == 1.0, "not burning out at to_go {to_go}");
                }
                t += dt;
            }
            let b = burst_at.expect("no burst");
            assert!(b > 0.05 && b < 0.6, "burst at {b}");
            let c = committed_at.expect("never committed");
            assert!((3.45..=3.95).contains(&c), "committed at {c}");
        }
    }
}
