//! A cruising car in the world: mcBrainCruising (`cruising.rs`) with its routines on the port's rails (`cruiserail.rs`,
//! `railroute.rs`), the ambient traffic's ghost (`GhostTraffic`, the hooks of the traffic in
//! side-session note cruise-ghost-hooks-spec.md) and the car (`CruiseCar`). docs/reference/ai/brain_cruising.md section 3.

use crate::brain::Brain;
use crate::cruiserail::{Drive, Frame, RailGoals, RailPos, Rails};
use crate::cruising::{CruiseEnv, Cruising, FollowCar, Ghost};
use crate::vehicles::parked::Mwc;
use glam::Vec3;

/// The ambient request the brain submits (F-25539's fill): the route, its start lane and phase, SetSpeed (req+68 flag
/// 0x40, when +1664 > 0) and SetDestSpeed (req+76 flag 0x20, when +1668 > -1).
pub struct GhostRequest {
    pub owner: u32,
    pub route: crate::railroute::Route,
    pub start: (u32, f32),
    pub set_speed: Option<f32>,
    pub dest_speed: Option<f32>,
}

/// What state 5 reads of a live ghost: its matrix (rows a, b, c, place), its rail record's lane and phase, its speed.
#[derive(Clone, Copy, Debug)]
pub struct GhostView {
    pub matrix: [Vec3; 4],
    pub rail: Option<(u32, f32)>,
    pub speed: f32,
}

/// The ambient traffic's side.
pub trait GhostTraffic {
    /// the ambient manager is there (F-12926 / the teleport test it)
    fn present(&self) -> bool;
    fn request_ghost(&mut self, r: GhostRequest) -> i32;
    /// a pending request cancelled (status 7, no event)
    fn cancel_request(&mut self, id: i32);
    /// a ghost let go by the brain (reason 5)
    fn release_ghost(&mut self, ghost: u32);
    fn ghost(&self, ghost: u32) -> Option<GhostView>;
    /// the shared ambient random numbers (the shared MWC of the rail walk's next-rail pick, F-20639)
    fn rng(&mut self) -> &mut Mwc;
}

/// The car's side: its matrix (rows a, b, c, place), velocity, yaw rate, the vehicle tune's +48, the speed the rail goals
/// read (car +224), the teleport's set (the racer's pose set: matrix, velocity, zero angular velocity), the nav-mesh test of
/// DriveToNearestRail and the ground probe of the placement.
pub trait CruiseCar {
    fn matrix(&self) -> Frame;
    fn vel(&self) -> Vec3;
    fn yaw_rate(&self) -> f32;
    fn tune48(&self) -> f32;
    fn speed(&self) -> f32;
    fn set_pose(&mut self, m: &Frame, vel: Vec3);
    fn on_mesh(&self, p: Vec3) -> bool;
    fn probe(&mut self, from: Vec3, to: Vec3) -> Option<(Vec3, Vec3)>;
    /// the reset position (racer +192)
    fn reset_pos(&self) -> Vec3;
    /// the live engine tune's idle rpm (+8) and engage rpm (+20), [[car+92]+144]
    fn idle_rpm(&self) -> f32;
    fn engage_rpm(&self) -> f32;
    fn set_engage_rpm(&mut self, rpm: f32);
    /// the car sim's computer-driven bytes +816 / +817
    fn set_computer_driven(&mut self, on: bool);
}

/// What drives the car this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Pedals {
    /// states 1 / 2: the racing brain's own output (racing slot 11)
    Racing,
    /// the others: the vehicle controller F-28541 with the curvature +1680 and speed +1676 demands (slot 11)
    Demand { steer: f32, speed: f32 },
}

/// A cruising car's brain (the racing brain it is in states 1 / 2 stays the car's own, passed in).
pub struct Cruiser {
    pub owner: u32,
    pub brain: Cruising,
    /// the reset position (racer +192, Racer_InitializeResetPosition)
    pub reset_pos: Vec3,
    /// the game-frame clock of the update (brain::frame_due) and slot 11's input, held between game frames (None:
    /// not run yet)
    pub frame_acc: f32,
    pub held: Option<crate::physics::CarInput>,
    /// the frame time the follow divides by (the image's word at the follow, 1/60)
    pub k: f32,
    /// the car's engage rpm saved by the enable (+1692; 1000 from the constructor)
    pub saved_engage: f32,
}

/// The world for one update of one cruiser.
pub struct World<'a, 'r, T: GhostTraffic, C: CruiseCar> {
    pub rails: &'a Rails<'a>,
    pub traffic: &'a mut T,
    pub car: &'a mut C,
    pub racing: &'a mut Brain,
    pub owner: u32,
    pub k: f32,
    pub racer_ran: bool,
    /// the racing brain's update (F-13552) on the car, run where the cruising update calls it (None: not run here)
    pub racer: Option<&'a mut (dyn FnMut(&mut T, &mut Brain) + 'r)>,
}

/// The brain's rail (+1700 / +1704) as the rail helpers take it: `Cruising::rail` holds the lane + 1, 0 for none.
fn rail_of(b: &Cruising) -> RailPos {
    RailPos { lane: b.rail.checked_sub(1), t: b.rail_t }
}

fn set_rail(b: &mut Cruising, rp: RailPos) {
    b.rail = rp.lane.map_or(0, |l| l + 1);
    b.rail_t = rp.t;
}

impl<T: GhostTraffic, C: CruiseCar> World<'_, '_, T, C> {
    /// F-12926: out of the traffic - a pending request cancelled (state 3), the ghost let go (state 5).
    fn remove_from_traffic(&mut self, b: &mut Cruising) {
        if self.traffic.present() {
            match b.state {
                3 => self.traffic.cancel_request(b.request),
                5 => self.traffic.release_ghost(b.ghost),
                _ => {}
            }
        }
        b.ghost = 0;
        b.request = -1;
    }

    fn apply(&mut self, g: &RailGoals) {
        self.racing.rail_goals(&g.goals, g.max_speed, g.final_heading, g.final_speed);
    }
}

impl<T: GhostTraffic, C: CruiseCar> CruiseEnv for World<'_, '_, T, C> {
    /// F-90476: out of the traffic, the placement F-97236 on the rail nearest `p` (any heading), the speed SetSpeed or
    /// the rail's along the rail, the car set there.
    fn teleport(&mut self, b: &mut Cruising, p: Vec3) -> bool {
        self.remove_from_traffic(b);
        if !self.traffic.present() {
            b.state = 4;
            return false;
        }
        let car = &mut *self.car;
        let mut rp = rail_of(b);
        let placed = self.rails.place(&mut rp, p, b.offset, &mut |a, z| car.probe(a, z));
        set_rail(b, rp);
        let Some(m) = placed else {
            b.state = 4;
            return false;
        };
        let speed = if b.speed > 0.0 { b.speed } else { rp.lane.map_or(0.0, |l| self.rails.speed(l)) };
        self.car.set_pose(&m, m.c * -speed);
        true
    }

    /// Route + request (F-25539 in the decode): the route from the rail to the current destination, the request; none
    /// (no lane on the route): state 4. (idx -1: a request with no route - what the ambient side does with it is open.)
    fn route(&mut self, b: &mut Cruising) {
        let rp = rail_of(b);
        let Some(lane) = rp.lane else {
            b.state = 4;
            return;
        };
        let route = if b.idx != -1 {
            let r = crate::railroute::route(self.rails.net, self.rails.nav, lane, rp.t, b.dests[b.idx as usize & 63]);
            if r.nodes.is_empty() {
                b.state = 4;
                return;
            }
            r
        } else {
            crate::railroute::Route::default()
        };
        log::debug!("cruiser {}: route from lane {} t {:.4} to {:?} (lane {:?} t {:.4}): {:?}", self.owner, lane, rp.t, b.dests[b.idx.max(0) as usize & 63], route.dest_lane, route.dest_t, route.nodes.iter().map(|n| (n.rec, n.lanes.clone())).collect::<Vec<_>>());
        b.request = self.traffic.request_ghost(GhostRequest {
            owner: self.owner,
            route,
            start: (lane, rp.t),
            set_speed: (b.speed > 0.0).then_some(b.speed),
            dest_speed: (b.dest_speed > -1.0).then_some(b.dest_speed),
        });
        b.state = 3;
    }

    fn rail_goals(&mut self, b: &mut Cruising) -> bool {
        let m = self.car.matrix();
        let mut rp = rail_of(b);
        let g = self.rails.goals(&mut rp, m.pos, m.c, Some(self.car.speed()), b.offset, self.traffic.rng());
        set_rail(b, rp);
        let Some(g) = g else { return false };
        self.apply(&g);
        if let Some(c) = g.closest {
            b.closest = c;
        }
        true
    }

    fn racer_update(&mut self, _: &mut Cruising) {
        self.racer_ran = true;
        if let Some(f) = self.racer.as_mut() {
            f(self.traffic, self.racing);
        }
    }

    fn racer_reached(&mut self, _: &Cruising) -> bool {
        self.racing.route_done()
    }

    fn drive_to_rail(&mut self, b: &mut Cruising) {
        self.remove_from_traffic(b);
        let m = self.car.matrix();
        let w0 = (b.idx >= 0).then(|| b.dests[0]);
        let car = &*self.car;
        let ambient = self.traffic.present();
        let mut rp = rail_of(b);
        let d = self.rails.drive_to_nearest_rail(&mut rp, m.pos, m.c, Some(car.speed()), b.offset, w0, ambient, |p| car.on_mesh(p), self.traffic.rng());
        set_rail(b, rp);
        match d {
            Drive::Rail(g) => {
                self.apply(&g);
                if let Some(c) = g.closest {
                    b.closest = c;
                }
                b.state = 1;
            }
            Drive::Waypoint(g) => {
                self.apply(&g);
                b.state = 2;
            }
            Drive::Teleport => b.state = 4,
        }
    }

    /// F-19049: the teleport at the car's own place, then route + request.
    fn rejoin_here(&mut self, b: &mut Cruising) {
        let p = self.car.matrix().pos;
        if self.teleport(b, p) {
            self.route(b);
        }
    }

    /// F-12634: the nearest rail to the car within 45 degrees of its way, then route + request (none: state 4 - approx.,
    /// the failure branch is not read).
    fn attach_nearest(&mut self, b: &mut Cruising) {
        let m = self.car.matrix();
        let mut rp = rail_of(b);
        let found = self.rails.nearest(&mut rp, m.pos, -m.c).is_some();
        set_rail(b, rp);
        if found {
            self.route(b);
        } else {
            b.state = 4;
        }
    }

    fn rail_pose(&mut self, rail: u32, t: f32) -> (Vec3, Vec3) {
        self.rails.pose(&RailPos { lane: rail.checked_sub(1), t }, Vec3::ZERO).unwrap_or((Vec3::ZERO, Vec3::X))
    }

    /// rail slot 14 at the car's place (F-24415: 8 chords from 450 m, else 4; 10 halvings)
    fn rail_nearest_t(&mut self, rail: u32) -> f32 {
        let lane = rail.saturating_sub(1);
        let c = &self.rails.nav.curves[lane as usize];
        let n = if self.rails.length(lane) < 450.0 { 4 } else { 8 };
        crate::railroute::nearest(c, self.car.matrix().pos, n, 10).0
    }

    /// State 5: the ghost followed (cruising::follow); more than 10 m off it, DriveToNearestRail. (The streamed-out
    /// pose on the ghost is not used: the port's cruising cars are always physical.)
    fn follow_ghost(&mut self, b: &mut Cruising) {
        let Some(g) = self.traffic.ghost(b.ghost) else { return };
        let (p, fwd) = match g.rail {
            // (the rail's point and its tangent: `pose` gives the c row, minus the tangent)
            Some((lane, t)) => self.rails.pose(&RailPos { lane: Some(lane), t }, b.offset).map_or((g.matrix[3], Vec3::ZERO - g.matrix[2]), |(p, c)| (p, Vec3::ZERO - c)),
            None => (g.matrix[3], Vec3::ZERO - g.matrix[2]),
        };
        let m = self.car.matrix();
        let car = FollowCar { a: m.a, c: m.c, pos: m.pos, vel: self.car.vel(), yaw_rate: self.car.yaw_rate(), tune48: self.car.tune48() };
        if crate::cruising::follow(&mut b.follow, &car, p, fwd, g.speed, self.k).is_none() {
            self.drive_to_rail(b);
        }
    }

    fn physical(&self) -> bool {
        true
    }

    fn car_pos(&self) -> Vec3 {
        self.car.matrix().pos
    }

    fn last_goal(&self) -> Option<Vec3> {
        self.racing.last_goal()
    }

    fn reset_pos(&self) -> Vec3 {
        self.car.reset_pos()
    }
}

/// Slot 11 (F-95280) with a physical car outside states 1 / 2: the clutch engages at the idle rpm (engine tune +20 =
/// +8) before the vehicle controller runs.
pub fn slot11_engine<C: CruiseCar>(state: u32, physical: bool, car: &mut C) {
    if physical && state != 1 && state != 2 {
        let idle = car.idle_rpm();
        car.set_engage_rpm(idle);
    }
}

/// Slot 9's own part (F-22477): the engage rpm saved (+1692), the car computer-driven.
pub fn enable_car<C: CruiseCar>(saved: &mut f32, car: &mut C) {
    *saved = car.engage_rpm();
    car.set_computer_driven(true);
}

/// Slot 10's own part (F-35041): the saved engage rpm back, the car no longer computer-driven.
pub fn disable_car<C: CruiseCar>(saved: f32, car: &mut C) {
    car.set_engage_rpm(saved);
    car.set_computer_driven(false);
}

impl Cruiser {
    pub fn new(owner: u32, reset_pos: Vec3) -> Cruiser {
        Cruiser { owner, brain: Cruising::default(), reset_pos, frame_acc: 0.0, held: None, k: 1.0 / 60.0, saved_engage: 1000.0 }
    }

    /// One update (F-64718) and what drives the car after it (slot 11: the racing brain in states 1 / 2, else the
    /// demands).
    pub fn update<T: GhostTraffic, C: CruiseCar>(&mut self, rails: &Rails, traffic: &mut T, car: &mut C, racing: &mut Brain, racer: Option<&mut (dyn FnMut(&mut T, &mut Brain) + '_)>) -> Pedals {
        let mut w = World { rails, traffic, car, racing, owner: self.owner, k: self.k, racer_ran: false, racer };
        self.brain.update(&mut w);
        match self.brain.state {
            1 | 2 => Pedals::Racing,
            _ => {
                slot11_engine(self.brain.state, true, car);
                Pedals::Demand { steer: self.brain.follow.steer, speed: self.brain.follow.demand }
            }
        }
    }

    /// The brain enabled (slot 9, F-22477): after the racing brain's own enable, the car's engage rpm is saved and
    /// the car is computer-driven.
    pub fn enable<C: CruiseCar>(&mut self, car: &mut C) {
        enable_car(&mut self.saved_engage, car);
    }

    /// The brain disabled (slot 10, F-35041): the engage rpm back, the car no longer computer-driven, then the racing
    /// brain's own disable.
    pub fn disable<C: CruiseCar>(&mut self, car: &mut C) {
        disable_car(self.saved_engage, car);
    }

    /// The ambient system's answer to the request: the ghost made (status 0) or not.
    pub fn attached<T: GhostTraffic, C: CruiseCar>(&mut self, rails: &Rails, traffic: &mut T, car: &mut C, racing: &mut Brain, status: i32, ghost: Option<u32>) {
        let ghost = ghost.and_then(|id| traffic.ghost(id).map(|v| Ghost { id, rail: v.rail.map(|(l, t)| (l + 1, t)), speed: v.speed }));
        let ghost = ghost.as_ref();
        let mut w = World { rails, traffic, car, racing, owner: self.owner, k: self.k, racer_ran: false, racer: None };
        self.brain.attach(&mut w, status, ghost);
    }

    /// The ghost's release: `reason` 1 at its route's end, `rail` the last lane it rode (None: none).
    pub fn released<T: GhostTraffic, C: CruiseCar>(&mut self, rails: &Rails, traffic: &mut T, car: &mut C, racing: &mut Brain, reason: i32, rail: Option<u32>, ghost: u32) {
        let rail = rail.map_or(0, |l| l + 1);
        let mut w = World { rails, traffic, car, racing, owner: self.owner, k: self.k, racer_ran: false, racer: None };
        self.brain.event(&mut w, reason, rail, ghost);
    }
}

/// A car's cruising brain, as `AiDriver` holds it (a copy of the driver starts without one).
#[derive(Default)]
pub struct CruiserSlot(pub Option<Box<Cruiser>>);

impl Clone for CruiserSlot {
    fn clone(&self) -> Self {
        CruiserSlot(None)
    }
}

impl std::fmt::Debug for CruiserSlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "CruiserSlot({})", self.0.as_ref().map_or(-1, |c| c.brain.state as i32))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Car {
        idle: f32,
        engage: f32,
        driven: [bool; 2],
    }

    impl CruiseCar for Car {
        fn matrix(&self) -> Frame {
            unreachable!()
        }
        fn vel(&self) -> Vec3 {
            unreachable!()
        }
        fn yaw_rate(&self) -> f32 {
            unreachable!()
        }
        fn tune48(&self) -> f32 {
            unreachable!()
        }
        fn speed(&self) -> f32 {
            unreachable!()
        }
        fn set_pose(&mut self, _: &Frame, _: Vec3) {
            unreachable!()
        }
        fn on_mesh(&self, _: Vec3) -> bool {
            unreachable!()
        }
        fn probe(&mut self, _: Vec3, _: Vec3) -> Option<(Vec3, Vec3)> {
            unreachable!()
        }
        fn reset_pos(&self) -> Vec3 {
            unreachable!()
        }
        fn idle_rpm(&self) -> f32 {
            self.idle
        }
        fn engage_rpm(&self) -> f32 {
            self.engage
        }
        fn set_engage_rpm(&mut self, rpm: f32) {
            self.engage = rpm;
        }
        fn set_computer_driven(&mut self, on: bool) {
            self.driven = [on; 2];
        }
    }

    /// The original's slot 9 / 10 / 11 run offline (work/racerai/cenable): each line is the op (0 enable, 1 disable,
    /// 2 controller), the state, physical, idle, engage, saved, the two bytes before, then engage, saved and the bytes
    /// after.
    #[test]
    fn cruise_enable_matches_the_original() {
        let text = include_str!("testdata/cruise_enable_ref.txt");
        let mut n = 0;
        for line in text.lines() {
            let w: Vec<&str> = line.split_whitespace().collect();
            let u = |i: usize| w[i].parse::<u32>().unwrap();
            let f = |i: usize| f32::from_bits(u32::from_str_radix(w[i], 16).unwrap());
            let mut car = Car { idle: f(3), engage: f(4), driven: [u(6) != 0, u(7) != 0] };
            let mut saved = f(5);
            match u(0) {
                0 => enable_car(&mut saved, &mut car),
                1 => disable_car(saved, &mut car),
                _ => slot11_engine(u(1), u(2) != 0, &mut car),
            }
            assert_eq!(car.engage.to_bits(), u32::from_str_radix(w[8], 16).unwrap(), "{line}");
            assert_eq!(saved.to_bits(), u32::from_str_radix(w[9], 16).unwrap(), "{line}");
            assert_eq!(car.driven, [u(10) != 0, u(11) != 0], "{line}");
            n += 1;
        }
        assert_eq!(n, 300);
    }
}
