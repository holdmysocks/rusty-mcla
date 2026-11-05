//! The cruising racers in the world: a car whose `AiDriver` holds a `Cruiser` (cruiser.rs, mcBrainCruising) is
//! driven by it once a game frame. In states 1 / 2 the car's own racing brain drives (brain.rs, the racing slot 11);
//! in the others the vehicle controller F-28541 takes the brain's curvature and speed demands (slot 11, F-95280).
//! The ambient traffic's ghost answers (traffic.rs `GhostEvent`) go to their owner's brain. `setup` is
//! CruisingHookmanCore's set-up of the brain. docs/reference/ai/brain_cruising.md.

use crate::cruiser::{CruiseCar, Cruiser, Pedals, World};
use crate::cruiserail::{Frame, Rails};
use crate::physics::collision::WorldCollider;
use crate::physics::CarInput;
use crate::vehicles::{CarInstance, Vehicles};
use glam::Vec3;
use mcla_formats::rage::navmesh::NavMesh;

thread_local! {
    static RAILS: Option<&'static Rails<'static>> = make_rails();
}

/// The city's rails (navsys lanes, railnet tables), read once.
fn make_rails() -> Option<&'static Rails<'static>> {
    let root = crate::vehicles::traffic::DATA_ROOT.get()?;
    let res = crate::assets::load_resource_file(&root.join("resources/city/sc/0x9960fcd8.r66")).ok()?;
    let nav: &'static _ = Box::leak(Box::new(mcla_formats::rage::navsys::NavSys::parse(&res).ok()?));
    let net: &'static _ = Box::leak(Box::new(crate::railroute::RailNet::parse(&res)));
    Some(Box::leak(Box::new(Rails::new(&res, nav, net))))
}

pub fn rails() -> Option<&'static Rails<'static>> {
    RAILS.with(|r| *r)
}

/// The car as the brain reads it; what the brain does to it is kept and put on the car afterwards (the car sits in
/// the vehicles the ghost hooks also take).
struct CarSnap<'w> {
    m: Frame,
    vel: Vec3,
    yaw_rate: f32,
    tune48: f32,
    idle: f32,
    engage: f32,
    reset: Vec3,
    col: &'w WorldCollider,
    mesh: Option<&'static NavMesh>,
    pose: Option<(Frame, Vec3)>,
}

impl CarSnap<'_> {
    fn of<'w>(c: &CarInstance, col: &'w WorldCollider, reset: Vec3) -> CarSnap<'w> {
        let st = &c.state;
        let r = st.rot;
        let e = c.params().veh.as_ref().map(|v| (v.t.engine.s.idle_rpm, v.t.engine.s.engage_rpm));
        CarSnap {
            m: Frame { a: r * Vec3::X, b: r * Vec3::Y, c: r * Vec3::Z, pos: st.pos },
            vel: st.vel,
            yaw_rate: st.ang_vel.y,
            // ([veh+104]+48: the car's aivVehicleTuning float 12, the IdleSpeed of its first gear)
            tune48: c.ai.as_ref().and_then(|a| a.tune.as_ref()).map_or(0.0, |t| t.idle_speed[0]),
            idle: e.map_or(0.0, |e| e.0),
            engage: e.map_or(0.0, |e| e.1),
            reset,
            col,
            mesh: crate::offroad::mesh(),
            pose: None,
        }
    }
}

impl CruiseCar for CarSnap<'_> {
    fn matrix(&self) -> Frame {
        self.pose.as_ref().map_or(self.m, |p| p.0)
    }
    fn vel(&self) -> Vec3 {
        self.pose.as_ref().map_or(self.vel, |p| p.1)
    }
    fn yaw_rate(&self) -> f32 {
        self.yaw_rate
    }
    fn tune48(&self) -> f32 {
        self.tune48
    }
    fn speed(&self) -> f32 {
        self.vel().length()
    }
    fn set_pose(&mut self, m: &Frame, vel: Vec3) {
        self.pose = Some((*m, vel));
    }
    fn on_mesh(&self, p: Vec3) -> bool {
        self.mesh.is_some_and(|m| m.on_mesh(p))
    }
    fn probe(&mut self, from: Vec3, to: Vec3) -> Option<(Vec3, Vec3)> {
        let d = to - from;
        let len = d.length();
        let h = self.col.probe_front(from, d / len.max(1e-6), len)?;
        Some((h.point, h.normal))
    }
    fn reset_pos(&self) -> Vec3 {
        self.reset
    }
    fn idle_rpm(&self) -> f32 {
        self.idle
    }
    fn engage_rpm(&self) -> f32 {
        self.engage
    }
    // (the port's sim takes a computer-driven car's EngageRPM as its IdleRPM from `CarInput::ai`, which every input
    // below carries: the original's +816 / +817 bytes and slot 11's write to +20 together)
    fn set_engage_rpm(&mut self, _rpm: f32) {}
    fn set_computer_driven(&mut self, _on: bool) {}
}

/// The brain's teleport put on the car (F-86308's set: matrix, velocity, no spin, the chassis reset).
fn apply_pose(v: &mut Vehicles, i: usize, col: &WorldCollider, pose: Option<(Frame, Vec3)>) {
    let Some((m, vel)) = pose else { return };
    let c = &mut v.cars[i];
    if let Some(vt) = c.params().veh.clone() {
        crate::physics::vehsim::set_pose(&mut c.state, &vt, col, [m.a, m.b, m.c, m.pos], vel, Vec3::ZERO);
        c.pose_history.reset();
    }
}

fn take(v: &mut Vehicles, i: usize) -> Option<(Box<Cruiser>, Box<crate::brain::Brain>)> {
    let a = v.cars[i].ai.as_mut()?;
    let c = a.cruiser.0.take()?;
    match a.brain.0.take() {
        Some(b) => Some((c, b)),
        None => {
            a.cruiser.0 = Some(c);
            None
        }
    }
}

fn put(v: &mut Vehicles, i: usize, c: Box<Cruiser>, b: Box<crate::brain::Brain>) {
    if let Some(a) = v.cars[i].ai.as_mut() {
        a.cruiser.0 = Some(c);
        a.brain.0 = Some(b);
    }
}

/// The ambient system's answers since the last frame, each to its owner's brain (served in the manager's frame,
/// before the brains run).
pub fn events(v: &mut Vehicles, col: &WorldCollider) {
    let ev = v.drain_ghost_events();
    if ev.is_empty() {
        return;
    }
    let Some(rails) = rails() else { return };
    for e in ev {
        let owner = match e {
            crate::vehicles::traffic::GhostEvent::Attached { owner, .. } | crate::vehicles::traffic::GhostEvent::Released { owner, .. } => owner,
        };
        let Some(i) = v.cars.iter().position(|c| c.uid == owner) else { continue };
        let Some((mut cr, mut br)) = take(v, i) else { continue };
        log::debug!("cruiser {}: {:?}", owner, e);
        let mut car = CarSnap::of(&v.cars[i], col, cr.reset_pos);
        match e {
            crate::vehicles::traffic::GhostEvent::Attached { status, ghost, .. } => cr.attached(rails, v, &mut car, &mut br, status, ghost),
            crate::vehicles::traffic::GhostEvent::Released { ghost, reason, rail, .. } => cr.released(rails, v, &mut car, &mut br, reason, rail, ghost),
        }
        log::debug!("cruiser {}: event -> state {}, ghost {}, request {}", cr.owner, cr.brain.state, cr.brain.ghost, cr.brain.request);
        if let Some(g) = crate::cruiser::GhostTraffic::ghost(v, cr.brain.ghost) {
            let rp = g.rail.and_then(|(l, t)| rails.pose(&crate::cruiserail::RailPos { lane: Some(l), t }, Vec3::ZERO)).map(|p| p.0);
            log::debug!("cruiser {}: ghost at {:?} rail {:?} (point {:?}, {:.1} m) {:.1} m/s, car at {:?}", cr.owner, g.matrix[3], g.rail, rp, rp.map_or(-1.0, |p| p.distance(v.cars[i].state.pos)), g.speed, v.cars[i].state.pos);
        }
        let pose = car.pose.take();
        put(v, i, cr, br);
        apply_pose(v, i, col, pose);
    }
}

pub fn is_cruiser(c: &CarInstance) -> bool {
    c.ai.as_ref().is_some_and(|a| a.cruiser.0.is_some())
}

/// One frame of a cruising racer: the racer's update (F-64718, which runs the racing brain's update F-13552 in
/// states 1 / 2 where it calls it) once a game frame, then slot 11; the input is held over the chassis ticks between.
pub fn tick(v: &mut Vehicles, col: &WorldCollider, i: usize, dt: f32) -> CarInput {
    let (Some(rails), Some((mut cr, mut br))) = (rails(), take(v, i)) else {
        return CarInput { brake: 1.0, ai: true, ..Default::default() };
    };
    if let (Some(held), true) = (cr.held, dt > 0.0) {
        if !crate::brain::frame_due(&mut cr.frame_acc, dt) {
            put(v, i, cr, br);
            return CarInput { nitro: false, sst: false, ..held };
        }
    }
    let mut car = CarSnap::of(&v.cars[i], col, cr.reset_pos);
    let was = cr.brain.state;
    let mut out = None;
    let pedals = {
        let mut run = |v: &mut Vehicles, b: &mut crate::brain::Brain| {
            b.held = None;
            out = Some(crate::brain::run(v, i, b, None, dt));
        };
        cr.update(rails, v, &mut car, &mut br, Some(&mut run))
    };
    if was == 5 {
        if let Some(g) = crate::cruiser::GhostTraffic::ghost(v, cr.brain.ghost) {
            let rp = g.rail.and_then(|(l, t)| rails.pose(&crate::cruiserail::RailPos { lane: Some(l), t }, Vec3::ZERO)).map(|p| p.0);
            log::trace!("cruiser {}: follow rail {:?} {:?} ({:.1} m) {:.1} m/s, car {:?} {:.1} m/s, steer {:.3} demand {:.2}", cr.owner, g.rail, rp, rp.map_or(-1.0, |p| p.distance(v.cars[i].state.pos)), g.speed, v.cars[i].state.pos, v.cars[i].state.speed(), cr.brain.follow.steer, cr.brain.follow.demand);
        }
    }
    if cr.brain.state != was {
        log::debug!("cruiser {}: state {} -> {} at {:?}, {:.1} m/s, dest {} of {}, rail {} t {:.3}", cr.owner, was, cr.brain.state, v.cars[i].state.pos, v.cars[i].state.speed(), cr.brain.idx, cr.brain.count, cr.brain.rail, cr.brain.rail_t);
    }
    let pose = car.pose.take();
    let input = match pedals {
        // (states 1 / 2: the racing brain's slot 11 - its output of this update, or of its last one when the cruising
        // update did not run it)
        Pedals::Racing => out.or(br.held).unwrap_or(CarInput { brake: 1.0, ai: true, ..Default::default() }),
        // (written to the input as they are, the gearbox at first or above: max(gear, 2) - `CarInput::animated`)
        Pedals::Demand { steer, speed } => {
            let (steer, throttle, brake) = demand(&v.cars[i], &br, steer, speed);
            log::trace!("cruiser {}: slot 11 kappa {:.4} a {:.2} -> steer {:.3} throttle {:.3} brake {:.3}, gear {:?}, vf {:.2}", cr.owner, cr.brain.follow.steer, cr.brain.follow.demand, steer, throttle, brake, v.cars[i].state.veh.as_ref().map(|s| s.gear), v.cars[i].state.vel.dot(v.cars[i].state.forward()));
            CarInput { throttle, brake, steer, ai: true, animated: true, ..Default::default() }
        }
    };
    cr.held = Some(input);
    put(v, i, cr, br);
    apply_pose(v, i, col, pose);
    input
}

/// Slot 11 outside states 1 / 2 (F-95280): F-28541 on the car's aiv tables with the curvature demand (+1680) and
/// the speed demand (+1676), the car's velocity and forward, the boost terms, -9.8 x its gravity factor, the
/// drift / sync torque ratio (F-40927) and the gear max(gear, 2) - 2 (the port's numbering: max(gear, 1) - 1);
/// steer, throttle and brake go to the input as they are, the handbrake 0. The terms as brain::tick's BrainCar has
/// them.
fn demand(c: &CarInstance, br: &crate::brain::Brain, kappa: f32, a_cmd: f32) -> (f32, f32, f32) {
    let st = &c.state;
    let vt = c.params().veh.clone();
    let vs = st.veh.as_ref();
    let (drift, nitro, sling, grav) = match vt.as_deref() {
        Some(t) => (
            if t.sync_torque > 0.0 { t.t.gyro.drift_torque / t.sync_torque } else { 0.0 },
            [t.t.nitro.boost_exp, t.t.nitro.boost_amount, t.t.nitro.boost_time],
            [t.t.ss_turbo.boost_exp, t.t.ss_turbo.boost_amount, t.t.ss_turbo.use_time],
            t.gravity,
        ),
        None => (0.0, [0.0; 3], [0.0; 3], 1.4),
    };
    let boost = [vs.map_or(0.0, |s| s.nitro_level * nitro[2]), nitro[1], vs.map_or(0.0, |s| s.sst.level * sling[2]), sling[1]];
    let gear = vs.map_or(1, |s| s.gear).max(1) - 1;
    br.aiv.controller(kappa, a_cmd, drift, boost, -9.8 * grav, gear, st.vel, st.forward(), st.right())
}

/// CruisingHookmanCore's set-up of a man's brain (`script_obj/game/CruisingHookmen/0x0d18ab57` 0x197..0x379):
/// BrainFactory_CreateBrainCruising; with more than one point, SetCurrentDest(the first, `first`) and AppendDest
/// for the others in order from it, wrapping, then SetLooping(1); with the core's start flag (static 100: the man
/// put at his reset position) TeleportToNearestRail, else DriveToNearestRail; Racer_Activate (the enable). The car
/// needs its `ai.tune`.
pub fn setup(v: &mut Vehicles, col: &WorldCollider, i: usize, pts: &[Vec3], first: usize, at_reset: bool, reset_pos: Vec3) -> bool {
    let Some(rails) = rails() else { return false };
    let owner = v.cars[i].uid;
    let Some(tune) = v.cars[i].ai.as_ref().and_then(|a| a.tune.clone()) else { return false };
    let mut cr = Box::new(Cruiser::new(owner, reset_pos));
    // (mcBrainCruising's base: a racing brain with no SetupRace / BeginRace, and no nitro: its +236 = 0)
    let mut br = Box::new(crate::brain::Brain::police(tune));
    br.allow_nitro = false;
    let n = pts.len();
    if n > 1 {
        let mut k = first % n;
        cr.brain.set_current_dest(pts[k]);
        for _ in 2..=n {
            k = (k + 1) % n;
            cr.brain.append_dest(pts[k]);
        }
        cr.brain.set_looping(1);
    }
    let mut car = CarSnap::of(&v.cars[i], col, reset_pos);
    {
        let k = cr.k;
        let mut w = World { rails, traffic: v, car: &mut car, racing: &mut br, owner, k, racer_ran: false, racer: None };
        if at_reset {
            cr.brain.teleport_to_nearest_rail(&mut w);
        } else {
            cr.brain.drive_to_nearest_rail(&mut w);
        }
    }
    log::debug!("cruiser {}: set up, {} points from {}, {} -> state {}", owner, n, first, if at_reset { "teleport" } else { "drive" }, cr.brain.state);
    cr.enable(&mut car);
    let pose = car.pose.take();
    if let Some(a) = v.cars[i].ai.as_mut() {
        a.cruiser.0 = Some(cr);
        a.brain.0 = Some(br);
    }
    apply_pose(v, i, col, pose);
    true
}
