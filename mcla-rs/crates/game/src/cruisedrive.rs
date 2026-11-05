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
    /// each cruiser's hold on the traffic as its last update left it: owner -> (state, request, ghost)
    static HELD: std::cell::RefCell<std::collections::HashMap<u32, (u32, i32, u32)>> = Default::default();
}

fn note(cr: &Cruiser) {
    HELD.with(|h| h.borrow_mut().insert(cr.owner, (cr.brain.state, cr.brain.request, cr.brain.ghost)));
}

/// A cruising car the port removed (its own spawn / despawn of the men, roamers.rs - the original keeps the brain
/// while roaming): its brain's RemoveFromTraffic (F-12926: a pending request cancelled in state 3, the ghost let go
/// in state 5), so no ghost rides on for an owner that is gone.
fn removed(v: &mut Vehicles) {
    let gone: Vec<(u32, (u32, i32, u32))> = HELD.with(|h| {
        let mut h = h.borrow_mut();
        let gone: Vec<u32> = h.keys().copied().filter(|&o| !v.cars.iter().any(|c| c.uid == o && is_cruiser(c))).collect();
        gone.into_iter().filter_map(|o| h.remove(&o).map(|x| (o, x))).collect()
    });
    for (owner, (state, request, ghost)) in gone {
        if !crate::cruiser::GhostTraffic::present(v) {
            continue;
        }
        log::debug!("cruiser {owner}: car removed in state {state}: out of the traffic");
        match state {
            3 => crate::cruiser::GhostTraffic::cancel_request(v, request),
            5 => crate::cruiser::GhostTraffic::release_ghost(v, ghost),
            _ => {}
        }
    }
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
    removed(v);
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
        note(&cr);
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
    note(&cr);
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
            // (F-95280: -9.8 x the tune's BoundGravity; `VehTune::gravity` is already 9.8 x it)
            t.t.s.bound_gravity,
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
    note(&cr);
    if let Some(a) = v.cars[i].ai.as_mut() {
        a.cruiser.0 = Some(cr);
        a.brain.0 = Some(br);
    }
    apply_pose(v, i, col, pose);
    true
}

#[cfg(test)]
mod capture_tests {
    use crate::cruising::{follow, Follow, FollowCar};
    use glam::Vec3;
    use std::collections::HashMap;

    type Rec = HashMap<String, Vec<u32>>;

    /// The line's fields: `key=dec[hex,hex]` as the bit words, plain values as their own word (rail pointers etc.).
    fn parse(line: &str) -> Rec {
        let mut m = HashMap::new();
        for tok in line.split_whitespace() {
            let Some((k, v)) = tok.split_once('=') else { continue };
            let w = match (v.find('['), v.ends_with(']')) {
                (Some(i), true) => v[i + 1..v.len() - 1].split(',').map(|h| u32::from_str_radix(h, 16).unwrap_or(0)).collect(),
                _ => vec![u32::from_str_radix(v, 16).ok().or_else(|| v.parse::<i64>().ok().map(|x| x as u32)).unwrap_or(0)],
            };
            if let Ok(d) = v.parse::<i64>() {
                m.insert(format!("{k}_dec"), vec![d as u32]);
            }
            m.insert(k.to_string(), w);
        }
        m
    }
    fn f(r: &Rec, k: &str) -> f32 {
        f32::from_bits(r[k][0])
    }
    fn v3(r: &Rec, k: &str) -> Vec3 {
        let w = &r[k];
        Vec3::new(f32::from_bits(w[0]), f32::from_bits(w[1]), f32::from_bits(w[2]))
    }

    /// State 5 of the cruising brain (F-64718 -> the follow) against a cruising man recorded in the original
    /// (work/racerai/cruiserec/cruise_ref.txt, man 350): for each recorded update the follow from the previous
    /// update's way / speed / curvature, the car as recorded, the ghost's rail point at its t ([ghost+452]+208) on the
    /// lane the port's exact lane curve puts there, and the ghost's speed (+412); the speed demand +1676, the curvature
    /// demand +1680 and the rail curvature +1688 compared bit for bit.
    #[test]
    fn follow_matches_the_capture() {
        let path = std::env::var("MCLA_CRUISE_REF").unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../../../work/racerai/cruiserec/cruise_ref2.txt").to_string());
        let Ok(text) = std::fs::read_to_string(&path) else { return };
        let root = std::path::PathBuf::from(std::env::var("MCLA_DATA_DIR").unwrap_or_else(|_| "D:/MCLA/extracted/cache".into()));
        let res = crate::assets::load_resource_file(&root.join("resources/city/sc/0x9960fcd8.r66")).unwrap();
        let nav = mcla_formats::rage::navsys::NavSys::parse(&res).unwrap();
        let recs: Vec<Rec> = text.lines().filter(|l| l.starts_with("run=")).map(parse).filter(|r| r.contains_key("u1708")).collect();
        let jpath = std::path::Path::new(&path).with_file_name("junction_curves.txt");
        let junction: HashMap<u32, mcla_formats::rage::navsys::LaneCurve> = std::fs::read_to_string(jpath)
            .unwrap_or_default()
            .lines()
            .filter(|l| l.starts_with("rail="))
            .map(|l| {
                let m: HashMap<&str, &str> = l.split_whitespace().filter_map(|t| t.split_once('=')).collect();
                let h = |x: &str| f32::from_bits(u32::from_str_radix(x, 16).unwrap());
                let pts = m["pts"].split(';').map(|p| { let v: Vec<f32> = p.split(',').map(h).collect(); Vec3::new(v[0], v[1], v[2]) }).collect();
                let c = mcla_formats::rage::navsys::LaneCurve { degree: m["degree"].parse().unwrap(), pts, knots: m["knots"].split(',').map(h).collect(), t_range: h(m["trange"]), t_max: h(m["tmax"]) };
                (u32::from_str_radix(m["rail"], 16).unwrap(), c)
            })
            .collect();
        let (mut on_junction, mut j_eq, mut j_diff) = (0usize, 0usize, Vec::<String>::new());
        let key = |r: &Rec| (r["f1672"][0], r["f1676"][0], r["f1680"][0], r["f1688"][0], r["g1616"].clone());
                let (mut n, mut eqs, mut gated, mut skipped) = (0, [[0usize; 3]; 4], 0, 0);
        let (mut n_same, mut near, mut worst) = (0usize, [0usize; 2], [0f32; 2]);
        let mut first_diff = Vec::new();
        for w in recs.windows(2) {
            let (a, b) = (&w[0], &w[1]);
            if a["u1708"][0] != 5 || b["u1708"][0] != 5 || a["run"] != b["run"] || key(a) == key(b) || a["i1648"] != b["i1648"] {
                continue;
            }
            // (the ghost's rail [ghost+452]+188: a lane record, 112 bytes in lane order - cruiserec/rails.txt; -1: the
            // records without control points seen at junctions, not decoded - left out)
            let t = f(b, "gt208");
            let lane = b["lane188_dec"][0];
            // (a lane-change rail, vtable 0x820CD1BC: its slot 8 (F-32383) is the lanes' F-71868 on its own curve at +124 -
            // cruiserec/junction_curves.txt; the TCP rails, 0x820CD224, are not in the capture's dumps: left out)
            let jc = junction.get(&b["rail188"][0]);
            if jc.is_none() && (lane as i32 == -1 || lane as usize >= nav.curves.len()) {
                skipped += 1;
                continue;
            }
            if jc.is_some() {
                on_junction += 1;
            }
            let pos = v3(b, "mx_pos");
            let c = jc.unwrap_or_else(|| &nav.curves[lane as usize]);
            let p = crate::railroute::point(c, t) + v3(b, "g1632");
            let fwd = crate::railroute::deriv(c, t);
            // (the speed the update used is what it stored, +1672: the ghost is stepped after the brain in a frame)
            let s = f(b, "f1672");
            n += 1;
            if f(b, "f1676") == -100.0 {
                gated += 1;
            }
            let ta = f(a, "gt208");
            let same_lane = a["rail188"] == b["rail188"];
            for (k, (src, tt)) in [(b, t), (a, t), (b, ta), (a, ta)].into_iter().enumerate() {
                if k >= 2 && !same_lane {
                    continue;
                }
                let p = crate::railroute::point(c, tt) + v3(b, "g1632");
                let fwd = crate::railroute::deriv(c, tt);
                let car = FollowCar {
                    a: v3(src, "mx_r"),
                    c: v3(src, "mx_back"),
                    pos: v3(src, "mx_pos"),
                    vel: v3(src, "vel"),
                    yaw_rate: v3(src, "angv").y,
                    tune48: if f(b, "f1676") == -100.0 { 1e9 } else { 0.0 },
                };
                let mut st = Follow { way: v3(a, "g1616"), speed: f(a, "f1672"), curvature: f(a, "f1688"), steer: f(a, "f1680"), demand: f(a, "f1676") };
                follow(&mut st, &car, p, fwd, s, crate::brain::FRAME);
                let got = [st.demand.to_bits(), st.steer.to_bits(), st.curvature.to_bits()];
                let want = [b["f1676"][0], b["f1680"][0], b["f1688"][0]];
                let tally = &mut eqs[k];
                for i in 0..3 {
                    if got[i] == want[i] {
                        tally[i] += 1;
                    }
                }
                if k == 3 {
                    n_same += 1;
                    let e = [(st.demand - f32::from_bits(want[0])).abs(), (st.steer - f32::from_bits(want[1])).abs()];
                    near[0] += (e[0] < 1e-3) as usize;
                    near[1] += (e[1] < 1e-5) as usize;
                    worst = [worst[0].max(e[0]), worst[1].max(e[1])];
                    if jc.is_some() {
                        j_eq += (got[2] == want[2]) as usize;
                        if got[2] != want[2] && j_diff.len() < 8 {
                            j_diff.push(format!("run {:?} seq {:?} rail {:08X} t {tt} (a's rail {:08X}): rail curvature {} want {}", b["run"], b["seq"], b["rail188"][0], a["rail188"][0], st.curvature, f32::from_bits(want[2])));
                        }
                    }
                    if got != want && first_diff.len() < 4 {
                        first_diff.push(format!("seq {:?} lane {lane} t {tt}: got {:?} want {:?}", b["seq"], got.map(f32::from_bits), want.map(f32::from_bits)));
                    }
                }
            }
        }
        eprintln!("follow vs capture: {n} updates ({on_junction} on lane-change rails, {gated} gated, {skipped} on TCP rails / not dumped); [demand, curvature demand, rail curvature] equal with (car, ghost t) of (b, b) {:?}, (a, b) {:?}, (b, a) {:?}, (a, a) {:?}", eqs[0], eqs[1], eqs[2], eqs[3]);
        eprintln!("  car and ghost t of the update before (the brain runs before the ghost's step): {n_same} updates, rail curvature equal {}, speed demand within 1e-3 {} (worst {}), curvature demand within 1e-5 {} (worst {})", eqs[3][2], near[0], worst[0], near[1], worst[1]);
        for d in &first_diff {
            eprintln!("  {d}");
        }
        eprintln!("  on lane-change rails: rail curvature equal {j_eq}");
        for d in &j_diff {
            eprintln!("  {d}");
        }
        // (the reader samples the car after the tick, not where the brain reads it: the demands, which take the car's
        // place and velocity, agree to the sampling; the rail curvature, from the ghost alone, bit for bit)
        // (lanes: 813 of 813; the lane-change rails: 130 of 151 - the misses are small and inside one rail, whose curve
        // was dumped once, the first time it was seen [I: the original may reshape it later])
        let j_n = on_junction.min(n_same);
        assert!(n_same > 500 && (eqs[3][2] - j_eq) * 100 >= (n_same - j_n) * 99, "rail curvature on lanes");
        assert!(j_n > 100 && j_eq * 100 >= j_n * 80, "rail curvature on lane-change rails");
        // (lanes alone 797 of 815; with the lane-change rails (the left turn, steer to 1.0) 946 / 927 of 964; the rest
        // are pairs with an update missed between samples [I])
        assert!(near[0] * 100 >= n_same * 95 && near[1] * 100 >= n_same * 95, "demands");
    }

    /// Slot 11 of the cruising brain in state 5 (F-95280 -> F-28541) against the same capture: the curvature and
    /// speed demands of a recorded update, the car (velocity, rows) as recorded and its gear from the recorder file
    /// (transmission +20) give the steer, throttle and brake the input object holds (+152 / +156 / +160). The man's
    /// state of tune is not in the capture: every share 0..1 in hundredths is tried and the best one printed.
    #[test]
    fn pedals_match_the_capture() {
        let dir = std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../work/racerai/cruiserec"));
        let Ok(text) = std::fs::read_to_string(dir.join("cruise_ref2.txt")) else { return };
        let data = std::env::var("MCLA_DATA_DIR").unwrap_or_else(|_| "D:/MCLA/extracted/cache".into());
        let assets = crate::assets::Assets::new(&data);
        let car = crate::vehicle_model::load_car(&assets, &std::path::Path::new(&data).join("resources/vehicle/vp_dge_challenger_70")).unwrap();
        let (Some(at), Some(st)) = (car.aitune.clone(), car.simtune.clone()) else { panic!("tunes") };
        let recs: Vec<Rec> = text.lines().filter(|l| l.starts_with("run=")).map(parse).filter(|r| r.contains_key("u1708")).collect();
        let files: Vec<Vec<u8>> = ["pc.bin", "p.0.bin", "p.0.bin"].iter().map(|n| std::fs::read(dir.join(n)).unwrap_or_default()).collect();
        let gear_at = |run: usize, tick: i64| -> Option<i32> {
            let d = files.get(run)?;
            if d.len() < 16 || tick < 0 {
                return None;
            }
            let size = u32::from_le_bytes(d[8..12].try_into().unwrap()) as usize;
            let o = 16 + tick as usize * size + 8 + 1024 + 160 + 20;
            d.get(o..o + 4).map(|b| u32::from_be_bytes(b.try_into().unwrap()) as i32)
        };
        let mut best = (0usize, 0.0f32, [0usize; 3], 0usize, Vec::new(), [0usize; 3]);
        for pc in 0..=100 {
            let share = pc as f32 / 100.0;
            let aiv = crate::aiv::Aiv::new(std::sync::Arc::new(mcla_formats::rage::aitune::AiTune::lerp(&at.base, &at.mods, share)));
            let vt = crate::physics::vehsim::VehTune::new(mcla_formats::rage::simtune::SimTune::lerp(&st.base, &st.mods, share));
            let drift = if vt.sync_torque > 0.0 { vt.t.gyro.drift_torque / vt.sync_torque } else { 0.0 };
            let (mut n, mut eq, mut near, mut diffs) = (0usize, [0usize; 3], [0usize; 3], Vec::new());
            for r in &recs {
                if r["u1708"][0] != 5 || r["i1648"][0] == 0 {
                    continue;
                }
                let run = r["run_dec"][0] as usize;
                let Some(gear) = gear_at(run, r["tick_dec"][0] as i32 as i64) else { continue };
                let back = v3(r, "mx_back");
                let right = v3(r, "mx_r");
                let (s, t, b) = aiv.controller(f(r, "f1680"), f(r, "f1676"), drift, [0.0, vt.t.nitro.boost_amount, 0.0, vt.t.ss_turbo.boost_amount], -9.8 * vt.t.s.bound_gravity, gear.max(2) - 2, v3(r, "vel"), Vec3::ZERO - back, right);
                let got = [s, t, b];
                let want = [f(r, "in152"), f(r, "in156"), f(r, "in160")];
                n += 1;
                for i in 0..3 {
                    let e = (got[i] - want[i]).abs();
                    if e < 1e-4 {
                        eq[i] += 1;
                    }
                    if e < 1e-2 {
                        near[i] += 1;
                    }
                }
                if got.iter().zip(&want).any(|(g, w)| (g - w).abs() >= 1e-4) && diffs.len() < 6 {
                    diffs.push(format!("run {run} seq {} gear {gear}: got {got:?} want {want:?}", r["seq_dec"][0]));
                }
            }
            if pc % 10 == 0 {
                eprintln!("  share {share}: within 1e-4 {eq:?}, within 1e-2 {near:?}");
            }
            if eq.iter().sum::<usize>() > best.2.iter().sum::<usize>() {
                best = (pc, share, eq, n, diffs, near);
            }
        }
        eprintln!("pedals vs capture: best share {} of 1: {} samples, [steer, throttle, brake] within 1e-4 {:?}", best.1, best.3, best.2);
        for d in &best.4 {
            eprintln!("  {d}");
        }
        // (2026-11-05: share 0.48 [I: fitted - the man's mod value is not in the capture]; 4488 / 3477 / 3137 of
        // 4979 within 1e-4, 4978 / 4928 / 4971 within 1e-2: the car is sampled after the tick, not where slot 11
        // read it)
        let (n, eq, near) = (best.3, best.2, best.5);
        assert!(n > 4000 && (best.0 as i32 - 48).abs() <= 2, "share {}", best.1);
        assert!(eq[0] * 100 >= n * 85, "steer");
        assert!(near[1] * 100 >= n * 98 && near[2] * 100 >= n * 98, "pedals");
    }
}
