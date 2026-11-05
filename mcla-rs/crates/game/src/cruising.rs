//! mcBrainCruising (docs/reference/ai/brain_cruising.md): the brain of the scripted cruising cars. Normally the car is
//! handed to the ambient traffic as a ghost on the lane rails and the physical car follows it; far off its ghost or off
//! the rails it drives with the racing brain to the nearest rail.

use crate::aiv::{ang_xz, ang_xz_unit, norm_or};
use crate::physics::exact::{fma, guest_atan2};
use glam::Vec3;

/// The brain's own fields the scripts set (brain_cruising.md section 1): the destinations (+560, 16 B each, count
/// +1584), the current one (+1656, -1: none), looping (+1660), the offset in the rail's frame (+1632), SetSpeed
/// (+1664) and SetDestSpeed (+1668), and the state (+1708).
#[derive(Clone, Debug)]
pub struct Cruising {
    pub dests: [Vec3; 64],
    pub count: i32,
    pub idx: i32,
    pub looping: bool,
    pub offset: Vec3,
    pub speed: f32,
    pub dest_speed: f32,
    pub state: u32,
    pub follow: Follow,
    /// the ambient ghost followed (+1648, 0: none) and the ambient request (+1652, -1: none)
    pub ghost: u32,
    pub request: i32,
    /// state 1: the closest the car has been to its last racing goal (+1696)
    pub closest: f32,
    /// the current rail (+1700) and its parameter (+1704); the ghost's rail point at the attach (+1600)
    pub rail: u32,
    pub rail_t: f32,
    pub rail_pt: Vec3,
}

impl Default for Cruising {
    /// The constructor's values (+1664 = +1668 = -1, the index -1).
    fn default() -> Self {
        Cruising { dests: [Vec3::ZERO; 64], count: 0, idx: -1, looping: false, offset: Vec3::ZERO, speed: -1.0, dest_speed: -1.0, state: 0, follow: Follow::default(), ghost: 0, request: -1, closest: 1e8, rail: 0, rail_t: 0.0, rail_pt: Vec3::ZERO }
    }
}

impl Cruising {
    /// BrainCruising_AppendDest (F-37550): onto the end (no bound: the script never passes 64), the current one at
    /// least the first.
    pub fn append_dest(&mut self, p: Vec3) {
        self.dests[self.count as usize & 63] = p;
        self.count += 1;
        if self.idx < 0 {
            self.idx = 0;
        }
    }
    /// BrainCruising_SetCurrentDest (F-51609): the list is this one point, the current one it.
    pub fn set_current_dest(&mut self, p: Vec3) {
        self.count = 1;
        self.dests[0] = p;
        self.idx = 0;
    }
    /// BrainCruising_SetLooping (F-92434)
    pub fn set_looping(&mut self, on: i32) {
        self.looping = on != 0;
    }
    /// BrainCruising_SetOffset (F-18127)
    pub fn set_offset(&mut self, v: Vec3) {
        self.offset = v;
    }
    /// BrainCruising_SetSpeed (F-47943)
    pub fn set_speed(&mut self, v: f32) {
        self.speed = v;
    }
    /// BrainCruising_SetDestSpeed (F-20377)
    pub fn set_dest_speed(&mut self, v: f32) {
        self.dest_speed = v;
    }
    /// BrainCruising_HasReachedGoal (F-67268): state 6 (past the last destination).
    pub fn has_reached_goal(&self) -> bool {
        self.state == 6
    }
    /// BrainCruising_GetCurrentDestinationIndex (F-76171; -1 without a brain).
    pub fn current_dest(b: Option<&Cruising>) -> i32 {
        b.map_or(-1, |b| b.idx)
    }
}

/// The ghost the ambient traffic hands over: its id, its rail record's rail and parameter (+452 -> +188, +208; None
/// when either is missing) and its speed (+412).
#[derive(Clone, Copy, Debug)]
pub struct Ghost {
    pub id: u32,
    pub rail: Option<(u32, f32)>,
    pub speed: f32,
}

/// What the brain's state logic asks of the world, one method per routine it calls (brain_cruising.md section 3).
pub trait CruiseEnv {
    /// the teleport F-90476 onto the rail nearest `p` (remove from traffic, place, velocity); false: nowhere (it sets state 4)
    fn teleport(&mut self, b: &mut Cruising, p: Vec3) -> bool;
    /// route + request (F-25539): a rail route to the current destination and the ambient request (state 3, or 4)
    fn route(&mut self, b: &mut Cruising);
    /// two racing goals on the nearest rail ahead (F-90991)
    fn rail_goals(&mut self, b: &mut Cruising) -> bool;
    /// the racing brain's update (F-13552)
    fn racer_update(&mut self, b: &mut Cruising);
    /// HasReachedGoal of the racing brain (F-43234)
    fn racer_reached(&mut self, b: &Cruising) -> bool;
    /// DriveToNearestRail (F-12584)
    fn drive_to_rail(&mut self, b: &mut Cruising);
    /// the teleport at the car's own place, then route + request (F-19049)
    fn rejoin_here(&mut self, b: &mut Cruising);
    /// the nearest rail to the car within 45 degrees, then route + request (F-12634)
    fn attach_nearest(&mut self, b: &mut Cruising);
    /// a rail's point and tangent at `t` (rail slot 8)
    fn rail_pose(&mut self, rail: u32, t: f32) -> (Vec3, Vec3);
    /// the parameter of a rail's point nearest the car (rail slot 14)
    fn rail_nearest_t(&mut self, rail: u32) -> f32;
    /// state 5 (`follow`, or the pose on the ghost without a physical car)
    fn follow_ghost(&mut self, b: &mut Cruising);
    /// the racer has a physical car (racer +48)
    fn physical(&self) -> bool;
    fn car_pos(&self) -> Vec3;
    /// the racing brain's last goal, None without goals (+176 / +180)
    fn last_goal(&self) -> Option<Vec3>;
    /// the racer's reset position (+192)
    fn reset_pos(&self) -> Vec3;
}

impl Cruising {
    /// The next destination after one is reached: on (wrapping when looping); past the last: state 6, none.
    fn next_dest(&mut self, env: &mut impl CruiseEnv) {
        self.idx += 1;
        if self.looping && self.idx >= self.count {
            self.idx = 0;
        }
        if self.idx < self.count {
            env.route(self);
        } else {
            self.count = 0;
            self.state = 6;
            self.idx = -1;
        }
    }

    /// F-64718, the update.
    pub fn update(&mut self, env: &mut impl CruiseEnv) {
        match self.state {
            4 => {
                if self.idx != -1 {
                    let p = self.dests[self.idx as usize & 63];
                    if env.teleport(self, p) {
                        self.next_dest(env);
                    }
                } else {
                    let p = env.reset_pos();
                    if env.teleport(self, p) {
                        env.route(self);
                    }
                }
            }
            1 => {
                if !env.physical() {
                    env.rejoin_here(self);
                    return;
                }
                let d2 = env.last_goal().map_or(0.0, |g| {
                    let d = env.car_pos() - g;
                    fma(d.x, d.x, fma(d.z, d.z, d.y * d.y)).sqrt()
                });
                if d2 > self.closest + 1.0 {
                    if !env.rail_goals(self) {
                        self.state = 4;
                        return;
                    }
                } else {
                    self.closest = if d2 - self.closest >= 0.0 { self.closest } else { d2 };
                }
                env.racer_update(self);
                if env.racer_reached(self) {
                    env.route(self);
                }
            }
            2 => {
                if !env.physical() {
                    env.rejoin_here(self);
                } else if env.rail_goals(self) {
                    self.state = 1;
                } else {
                    env.racer_update(self);
                }
            }
            5 => env.follow_ghost(self),
            3 => {
                self.follow.steer = 0.0;
                self.follow.demand = 0.0;
            }
            _ => {
                self.follow.steer = 0.0;
                self.follow.demand = -100.0;
            }
        }
    }

    /// The attach (vt 12, F-26049): the ambient system has made the ghost (`status` 0) or failed.
    pub fn attach(&mut self, env: &mut impl CruiseEnv, status: i32, ghost: Option<&Ghost>) {
        self.request = -1;
        match ghost.filter(|_| status == 0).and_then(|g| g.rail.map(|r| (g, r))) {
            Some((g, (rail, t))) => {
                self.ghost = g.id;
                let (p, tan) = env.rail_pose(rail, t);
                self.rail_pt = p;
                self.follow.way = tan;
                self.state = 5;
                self.follow.speed = g.speed;
            }
            None => self.state = 4,
        }
    }

    /// The ghost's release (vt 15, F-95102): `reason` 1 at its route's end, 5 let go by the brain; `rail` where it
    /// ended (0: none). Ignored for another ghost.
    pub fn event(&mut self, env: &mut impl CruiseEnv, reason: i32, rail: u32, ghost: u32) {
        if ghost != self.ghost {
            return;
        }
        self.ghost = 0;
        match reason {
            1 => {
                self.idx += 1;
                if self.looping && self.idx >= self.count {
                    self.idx = 0;
                }
                if self.idx < self.count {
                    if rail == 0 {
                        env.attach_nearest(self);
                    } else {
                        self.rail = rail;
                        self.rail_t = env.rail_nearest_t(rail);
                        env.route(self);
                    }
                } else {
                    self.count = 0;
                    self.state = 6;
                    self.idx = -1;
                }
            }
            5 => {
                if self.state == 5 {
                    self.state = 0;
                }
            }
            _ => self.state = 4,
        }
    }

    /// BrainCruising_TeleportToNextDestination's step (F-11718): onto the current destination's rail, then on.
    pub fn teleport_next(&mut self, env: &mut impl CruiseEnv) {
        let p = self.dests[self.idx as usize & 63];
        if env.teleport(self, p) {
            self.next_dest(env);
        }
    }

    /// BrainCruising_TeleportToNearestRail (F-35641): the teleport F-90476 onto the rail nearest the car's own place, then route + request (F-25539).
    pub fn teleport_to_nearest_rail(&mut self, env: &mut impl CruiseEnv) {
        let p = env.car_pos();
        if env.teleport(self, p) {
            env.route(self);
        }
    }

    /// BrainCruising_DriveToNearestRail (F-92832: F-12584)
    pub fn drive_to_nearest_rail(&mut self, env: &mut impl CruiseEnv) {
        env.drive_to_rail(self);
    }
}

/// The physical car as state 5 reads it: its matrix rows a and c and its place (the matrix block, F-95017's car
/// branch), its velocity (F-98427: the sim's +272), the sim's yaw rate (+292) and the vehicle tune's +48 (x 0.25:
/// the slowest ghost it follows).
#[derive(Clone, Copy, Debug)]
pub struct FollowCar {
    pub a: Vec3,
    pub c: Vec3,
    pub pos: Vec3,
    pub vel: Vec3,
    pub yaw_rate: f32,
    pub tune48: f32,
}

/// What state 5 keeps between updates: the ghost's flat way (+1616), its speed (+1672), the rail's curvature
/// (+1688) and the demands to the vehicle controller (F-28541, aiv.rs::pedal_map): curvature +1680, speed +1676.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Follow {
    pub way: Vec3,
    pub speed: f32,
    pub curvature: f32,
    pub steer: f32,
    pub demand: f32,
}

impl Default for Follow {
    fn default() -> Self {
        Follow { way: Vec3::ZERO, speed: 0.0, curvature: 0.0, steer: 0.0, demand: -100.0 }
    }
}

/// F-64718 in state 5 with a physical car: follow the ghost at `p` going `fwd` (its matrix's -c, or the rail's
/// tangent) at `s` m/s; `k` the frame's time (the image's word 1/60). None: the ghost is more than 10 m away in XZ
/// (F-12584 DriveToNearestRail takes over; the way and speed are still kept, as the original stores them after it).
pub fn follow(st: &mut Follow, car: &FollowCar, p: Vec3, fwd: Vec3, s: f32, k: f32) -> Option<()> {
    let t = norm_or(Vec3::new(fwd.x, 0.0, fwd.z), 1e-5, Vec3::X);
    let f = Vec3::ZERO - car.c;
    let d = p - car.pos;
    let near = !(fma(d.z, d.z, d.x * d.x) > 100.0);
    if near {
        let v = car.vel;
        let vf = -fma(v.x, car.c.x, fma(car.c.z, v.z, car.c.y * v.y));
        if s > car.tune48 * 0.25 {
            let lm = if 5.0 - s >= 0.0 { 5.0 } else { s };
            let l = lm * 0.5;
            let ks = k * s;
            let a1 = ang_xz(f, st.way);
            let ad = fma(d.x, car.a.x, fma(car.a.y, d.y, car.a.z * d.z));
            let a0 = guest_atan2(-ad as f64, l as f64) as f32;
            let fxz = norm_or(Vec3::new(f.x, 0.0, f.z), 1e-5, Vec3::X);
            let vd = norm_or(Vec3::new(v.x, 0.0, v.z), 1.0, fxz);
            let a2 = ang_xz_unit(vd, st.way);
            let dot = fma(vd.x, fxz.x, fma(vd.z, fxz.z, vd.y * fxz.y));
            let yaw = st.curvature - car.yaw_rate / lm;
            let a3 = ang_xz_unit(st.way, t);
            let kn = a3 / ks;
            st.curvature = kn;
            st.steer = ((a2 * dot + a0) + a1) / l + kn + yaw;
            let fd = fma(d.x, f.x, fma(f.y, d.y, d.z * f.z));
            st.demand = fma(fd, 2.0, (s - st.speed) / k) + (st.speed - vf);
        } else {
            st.demand = -100.0;
        }
    }
    st.speed = s;
    st.way = t;
    near.then_some(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// F-64718's state 5 run offline (work/racerai/cfollow): 20,000 cars and ghosts within 10 m (ghost without a rail:
    /// its matrix's place and -c), ghost speeds under and over the car's least, frame times 1/60, 1/30 and others,
    /// a degenerate way and an upright ghost for the fallbacks. Steer, speed demand, curvature, speed and way bit for bit.
    #[test]
    fn follow_is_the_originals() {
        let Ok(b) = std::fs::read("D:/MCLA/work/racerai/cfollow/cases.bin") else { return };
        let w = |r: &[u8], i: usize| f32::from_bits(u32::from_le_bytes(r[4 * i..4 * i + 4].try_into().unwrap()));
        let v = |r: &[u8], i: usize| Vec3::new(w(r, i), w(r, i + 1), w(r, i + 2));
        let mut bad = 0;
        for r in b.chunks_exact(52 * 4) {
            let car = FollowCar { a: v(r, 0), c: v(r, 8), pos: v(r, 12), vel: v(r, 38), yaw_rate: w(r, 41), tune48: w(r, 42) };
            let mut st = Follow { way: v(r, 33), speed: w(r, 36), curvature: w(r, 37), steer: f32::from_bits(0x7fc0_1234), demand: 0.0 };
            // (the steer is kept when the ghost is slow: the case's own output stands in for the old value)
            let slow = !(w(r, 32) > w(r, 42) * 0.25);
            if slow {
                st.steer = w(r, 44);
            }
            follow(&mut st, &car, v(r, 28), Vec3::ZERO - v(r, 24), w(r, 32), w(r, 43));
            let got = [st.steer, st.demand, st.curvature, st.speed, st.way.x, st.way.y, st.way.z].map(f32::to_bits);
            let want: [u32; 7] = std::array::from_fn(|i| w(r, 44 + i).to_bits());
            if got != want {
                if bad < 5 {
                    println!("port {got:08x?}\noriginal {want:08x?}");
                }
                bad += 1;
            }
        }
        assert!(b.len() / 208 >= 20000 && bad == 0, "{bad} differ");
    }

    /// The natives run offline on 400 brains, 50 random calls each (work/racerai/cnat): every field and result after
    /// every call equal.
    #[test]
    fn natives_are_the_originals() {
        let Ok(b) = std::fs::read("D:/MCLA/work/racerai/cnat/cases.bin") else { return };
        let w = |r: &[u8], i: usize| u32::from_le_bytes(r[4 * i..4 * i + 4].try_into().unwrap());
        let f = |r: &[u8], i: usize| f32::from_bits(w(r, i));
        let mut bad = 0;
        let mut br = Cruising::default();
        for (n, r) in b.chunks_exact(16 * 4 + 64 * 12).enumerate() {
            if n % 50 == 0 {
                br = Cruising::default();
            }
            let v = Vec3::new(f(r, 1), f(r, 2), f(r, 3));
            br.state = w(r, 5);
            let mut res = 0xdead_beef_u32;
            match w(r, 0) {
                0 => br.append_dest(v),
                1 => br.set_current_dest(v),
                2 => br.set_looping(w(r, 4) as i32),
                3 => br.set_offset(v),
                4 => br.set_speed(f(r, 4)),
                5 => br.set_dest_speed(f(r, 4)),
                6 => res = br.has_reached_goal() as u32,
                _ => res = Cruising::current_dest(Some(&br)) as u32,
            }
            let got = [res, br.count as u32, br.idx as u32, br.looping as u32, br.speed.to_bits(), br.dest_speed.to_bits(), br.offset.x.to_bits(), br.offset.y.to_bits(), br.offset.z.to_bits()];
            let want: [u32; 9] = std::array::from_fn(|i| w(r, 6 + i));
            let list = (0..64).all(|i| [br.dests[i].x, br.dests[i].y, br.dests[i].z].map(f32::to_bits) == [w(r, 16 + 3 * i), w(r, 17 + 3 * i), w(r, 18 + 3 * i)]);
            if got != want || !list {
                if bad < 5 {
                    println!("call {n} op {}: port {got:08x?} original {want:08x?} list {list}", w(r, 0));
                }
                bad += 1;
            }
        }
        assert!(b.len() / 832 >= 20000 && bad == 0, "{bad} differ");
    }

    /// The original's calls of a case, replayed: each method takes the next logged call (its id must match) and gives
    /// its logged result.
    struct Replay<'a> {
        r: &'a [u8],
        next: usize,
        bad: bool,
    }

    impl Replay<'_> {
        fn w(&self, i: usize) -> u32 {
            u32::from_le_bytes(self.r[4 * i..4 * i + 4].try_into().unwrap())
        }
        fn f(&self, i: usize) -> f32 {
            f32::from_bits(self.w(i))
        }
        fn v(&self, i: usize) -> Vec3 {
            Vec3::new(self.f(i), self.f(i + 1), self.f(i + 2))
        }
        /// the next call: its result and two arguments
        fn take(&mut self, id: u32) -> (u32, u32, u32) {
            let k = self.next;
            self.next += 1;
            if k >= self.w(6) as usize || self.w(44 + 4 * k) != id {
                self.bad = true;
                return (0, 0, 0);
            }
            (self.w(45 + 4 * k), self.w(46 + 4 * k), self.w(47 + 4 * k))
        }
    }

    impl CruiseEnv for Replay<'_> {
        fn teleport(&mut self, _: &mut Cruising, p: Vec3) -> bool {
            let (r, a, b) = self.take(1);
            if (a, b) != (p.x.to_bits(), p.z.to_bits()) {
                self.bad = true;
            }
            r != 0
        }
        fn route(&mut self, _: &mut Cruising) {
            self.take(2);
        }
        fn rail_goals(&mut self, _: &mut Cruising) -> bool {
            self.take(3).0 != 0
        }
        fn racer_update(&mut self, _: &mut Cruising) {
            self.take(4);
        }
        fn racer_reached(&mut self, _: &Cruising) -> bool {
            self.take(5).0 != 0
        }
        fn drive_to_rail(&mut self, _: &mut Cruising) {
            self.take(6);
        }
        fn rejoin_here(&mut self, _: &mut Cruising) {
            self.take(7);
        }
        fn attach_nearest(&mut self, _: &mut Cruising) {
            self.take(8);
        }
        fn rail_pose(&mut self, rail: u32, t: f32) -> (Vec3, Vec3) {
            let (_, a, b) = self.take(9);
            if (a, b) != (rail, t.to_bits()) {
                self.bad = true;
            }
            (self.v(26 + 12), self.v(26 + 15))
        }
        fn rail_nearest_t(&mut self, rail: u32) -> f32 {
            let (r, a, _) = self.take(10);
            if a != rail {
                self.bad = true;
            }
            f32::from_bits(r)
        }
        fn follow_ghost(&mut self, _: &mut Cruising) {
            self.take(11);
        }
        fn physical(&self) -> bool {
            self.w(4) != 0
        }
        fn car_pos(&self) -> Vec3 {
            self.v(108)
        }
        fn last_goal(&self) -> Option<Vec3> {
            (self.w(5) > 0).then(|| self.v(114))
        }
        fn reset_pos(&self) -> Vec3 {
            self.v(111)
        }
    }

    /// The state logic run offline (work/racerai/cstate): 20,000 cases of the update in every state but 5, the attach,
    /// the ghost event and TeleportToNextDestination, the callees logged stubs. The same calls in the same order with the
    /// same arguments, and every field after equal.
    #[test]
    fn state_logic_is_the_originals() {
        let Ok(b) = std::fs::read("D:/MCLA/work/racerai/cstate/cases.bin") else { return };
        let mut bad = 0;
        for r in b.chunks_exact(138 * 4) {
            let mut env = Replay { r, next: 0, bad: false };
            let fields = |e: &Replay, o: usize| -> [u32; 18] { std::array::from_fn(|i| e.w(o + i)) };
            let pre = fields(&env, 8);
            let mut br = Cruising::default();
            for i in 0..6 {
                br.dests[i] = env.v(120 + 3 * i);
            }
            let f = f32::from_bits;
            (br.count, br.ghost, br.request, br.idx, br.looping) = (pre[0] as i32, pre[1], pre[2] as i32, pre[3] as i32, pre[4] >> 24 != 0);
            (br.follow.speed, br.follow.demand, br.follow.steer, br.closest) = (f(pre[5]), f(pre[6]), f(pre[7]), f(pre[8]));
            (br.rail, br.rail_t, br.state) = (pre[9], f(pre[10]), pre[11]);
            br.rail_pt = Vec3::new(f(pre[12]), f(pre[13]), f(pre[14]));
            br.follow.way = Vec3::new(f(pre[15]), f(pre[16]), f(pre[17]));
            let (a1, a2, a3) = (env.w(1), env.w(2), env.w(3));
            match env.w(0) {
                0 => br.update(&mut env),
                1 => {
                    let g = (a2 != 0).then(|| Ghost { id: a2, rail: (a2 == 0x16000 && env.w(119) == 3).then(|| (0x17100, env.f(117))), speed: env.f(118) });
                    br.attach(&mut env, a1 as i32, g.as_ref())
                }
                2 => br.event(&mut env, a1 as i32, a2, a3),
                _ => br.teleport_next(&mut env),
            }
            let got = [
                br.count as u32, br.ghost, br.request as u32, br.idx as u32, (br.looping as u32) << 24, br.follow.speed.to_bits(), br.follow.demand.to_bits(),
                br.follow.steer.to_bits(), br.closest.to_bits(), br.rail, br.rail_t.to_bits(), br.state,
                br.rail_pt.x.to_bits(), br.rail_pt.y.to_bits(), br.rail_pt.z.to_bits(), br.follow.way.x.to_bits(), br.follow.way.y.to_bits(), br.follow.way.z.to_bits(),
            ];
            let want = fields(&env, 26);
            if got != want || env.bad || env.next != env.w(6) as usize {
                if bad < 5 {
                    println!("op {} pre {pre:x?}
port {got:x?}
orig {want:x?} calls {} of {} bad {}", env.w(0), env.next, env.w(6), env.bad);
                }
                bad += 1;
            }
        }
        assert!(b.len() / 552 >= 20000 && bad == 0, "{bad} differ");
    }
}
