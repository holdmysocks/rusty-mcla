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
}

/// What drives the car this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Pedals {
    /// states 1 / 2: the racing brain's own output (racing slot 11)
    Racing,
    /// the others: the vehicle controller F-28541 with the curvature +1680 and speed +1676 demands (slot 11)
    Demand { steer: f32, speed: f32 },
}

/// A cruising car's brain and the racing brain it is (states 1 / 2).
pub struct Cruiser {
    pub owner: u32,
    pub brain: Cruising,
    pub racing: Brain,
    /// the frame time the follow divides by (the image's word at the follow, 1/60)
    pub k: f32,
}

/// The world for one update of one cruiser.
pub struct World<'a, T: GhostTraffic, C: CruiseCar> {
    pub rails: &'a Rails<'a>,
    pub traffic: &'a mut T,
    pub car: &'a mut C,
    pub racing: &'a mut Brain,
    pub owner: u32,
    pub k: f32,
    pub racer_ran: bool,
}

/// The brain's rail (+1700 / +1704) as the rail helpers take it: `Cruising::rail` holds the lane + 1, 0 for none.
fn rail_of(b: &Cruising) -> RailPos {
    RailPos { lane: b.rail.checked_sub(1), t: b.rail_t }
}

fn set_rail(b: &mut Cruising, rp: RailPos) {
    b.rail = rp.lane.map_or(0, |l| l + 1);
    b.rail_t = rp.t;
}

impl<T: GhostTraffic, C: CruiseCar> World<'_, T, C> {
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

impl<T: GhostTraffic, C: CruiseCar> CruiseEnv for World<'_, T, C> {
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
            Some((lane, t)) => self.rails.pose(&RailPos { lane: Some(lane), t }, b.offset).unwrap_or((g.matrix[3], -g.matrix[2])),
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

impl Cruiser {
    pub fn new(owner: u32, racing: Brain) -> Cruiser {
        Cruiser { owner, brain: Cruising::default(), racing, k: 1.0 / 60.0 }
    }

    /// One update (F-64718) and what drives the car after it (slot 11: the racing brain in states 1 / 2, else the
    /// demands).
    pub fn update<T: GhostTraffic, C: CruiseCar>(&mut self, rails: &Rails, traffic: &mut T, car: &mut C) -> Pedals {
        let mut w = World { rails, traffic, car, racing: &mut self.racing, owner: self.owner, k: self.k, racer_ran: false };
        self.brain.update(&mut w);
        match self.brain.state {
            1 | 2 => Pedals::Racing,
            _ => Pedals::Demand { steer: self.brain.follow.steer, speed: self.brain.follow.demand },
        }
    }

    /// The ambient system's answer to the request: the ghost made (status 0) or not.
    pub fn attached<T: GhostTraffic, C: CruiseCar>(&mut self, rails: &Rails, traffic: &mut T, car: &mut C, status: i32, ghost: Option<u32>) {
        let ghost = ghost.and_then(|id| traffic.ghost(id).map(|v| Ghost { id, rail: v.rail.map(|(l, t)| (l + 1, t)), speed: v.speed }));
        let ghost = ghost.as_ref();
        let mut w = World { rails, traffic, car, racing: &mut self.racing, owner: self.owner, k: self.k, racer_ran: false };
        self.brain.attach(&mut w, status, ghost);
    }

    /// The ghost's release: `reason` 1 at its route's end, `rail` the last lane it rode (None: none).
    pub fn released<T: GhostTraffic, C: CruiseCar>(&mut self, rails: &Rails, traffic: &mut T, car: &mut C, reason: i32, rail: Option<u32>, ghost: u32) {
        let rail = rail.map_or(0, |l| l + 1);
        let mut w = World { rails, traffic, car, racing: &mut self.racing, owner: self.owner, k: self.k, racer_ran: false };
        self.brain.event(&mut w, reason, rail, ghost);
    }
}
