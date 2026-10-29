//! Police: law-breaking score, pull-over, pursuit, search and arrest.
//!
//! All numbers come from the game's own `tune/police/globals.xml`
//! (`mcPoliceTuningGlobals`); patrol cars start on the routes of
//! `tune/police/patrolpath.xml`. The cruiser is `vp_chv_police_96`.

use crate::props::Inst;
use crate::render::Gpu;
use crate::vehicles::Vehicles;
use crate::world::World;
use glam::Vec3;
use hashbrown::HashMap;
use mcla_formats::rage::navsys::{NavNode, NavSys};
use std::path::Path;

pub const COP_CAR: &str = "vp_chv_police_96";
const MPH: f32 = 2.236_94;

#[derive(Default)]
pub struct Tuning {
    v: HashMap<String, f32>,
    /// the `content="int_array"` members (`iLawFines`), in file order
    arrays: HashMap<String, Vec<i32>>,
}

impl Tuning {
    pub fn load(path: &Path) -> Tuning {
        let mut v = HashMap::new();
        let mut arrays = HashMap::new();
        if let Ok(t) = std::fs::read_to_string(path) {
            let mut open: Option<(String, Vec<i32>)> = None;
            for line in t.lines() {
                let l = line.trim();
                if let Some((name, list)) = open.as_mut() {
                    if l.starts_with("</") {
                        arrays.insert(std::mem::take(name), std::mem::take(list));
                        open = None;
                    } else if let Ok(n) = l.parse::<i32>() {
                        list.push(n);
                    }
                    continue;
                }
                if l.starts_with('<') && l.ends_with("content=\"int_array\">") {
                    let name = l[1..].split_whitespace().next().unwrap_or("").to_string();
                    open = Some((name, Vec::new()));
                    continue;
                }
                let Some(rest) = l.strip_prefix('<') else { continue };
                let Some(sp) = rest.find(" value=\"") else { continue };
                let name = &rest[..sp];
                let val = &rest[sp + 8..];
                let Some(end) = val.find('"') else { continue };
                if let Ok(f) = val[..end].parse::<f32>() {
                    v.insert(name.to_string(), f);
                }
            }
        }
        Tuning { v, arrays }
    }

    /// One entry of an int array member; the members the file lists fewer values for read 0.
    pub fn array_at(&self, key: &str, i: usize) -> i32 {
        self.arrays.get(key).and_then(|a| a.get(i)).copied().unwrap_or(0)
    }

    pub fn get(&self, key: &str, default: f32) -> f32 {
        self.v.get(key).copied().unwrap_or(default)
    }
}

/// `<Points content="vector3_array">` blocks of patrolpath.xml.
fn load_patrol_paths(path: &Path) -> Vec<Vec<Vec3>> {
    let mut out = Vec::new();
    let Ok(t) = std::fs::read_to_string(path) else { return out };
    let mut cur: Option<Vec<Vec3>> = None;
    for line in t.lines() {
        let l = line.trim();
        if l.starts_with("<Points") {
            cur = Some(Vec::new());
        } else if l.starts_with("</Points") {
            if let Some(c) = cur.take() {
                if c.len() >= 2 {
                    out.push(c);
                }
            }
        } else if let Some(c) = cur.as_mut() {
            let f: Vec<f32> = l.split_whitespace().filter_map(|x| x.parse().ok()).collect();
            if f.len() == 3 {
                c.push(Vec3::new(f[0], f[1], f[2]));
            }
        }
    }
    out
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Heat {
    None,
    /// a cruiser wants the player to stop
    PullOver,
    Pursuit,
    /// lost sight of the player; the pursuit ends when the search runs out
    Search,
    Busted,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Event {
    PullOver,
    Pursuit,
    Escaped,
    /// stopped for the cruiser: fine
    Ticket(i64),
    /// arrested during a pursuit: fine
    Busted(i64),
    /// the arrest's hold (fArrestStateTime) is over: the tactic succeeded and the manager posts COP_ARREST (F-22530),
    /// which runs CopArrest.sco - its scene
    ArrestScene,
}

/// A cruiser's state in the pull-over or pursuit tactic (GitHub #17; docs/reference/hookmen/pursuit_tactics.md).
/// Pursuit (F-68032's FSM): CatchUp, AlongSide, Block, Halt (Delay is `flee_hold`, Search `Heat::Search`, Arrest the
/// port's busted test); pull-over (F-36876's): CatchUp, AvoidCollision (`Avoid`), Halt.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum Tac {
    #[default]
    CatchUp,
    AlongSide,
    Block,
    Halt,
    Avoid,
}

/// One cruiser's tactic state and its brain's goal plan (F-74140 / F-67303).
#[derive(Clone, Copy, Debug, Default)]
struct CopTac {
    st: Tac,
    /// seconds in `st`
    t: f32,
    /// the side of the target it drives beside: true = the target's right (F-14848)
    side: bool,
    side_t: f32,
    /// AlongSide: seconds out of reach (> 1.5 x fAlongSideDist)
    far_t: f32,
    /// pull-over Halt: seconds the gap has been growing while the target moves
    grow_t: f32,
    d_prev: f32,
    /// the plan's countdown (+588), its two goals and the speed to arrive with (-1: full pace)
    replan: f32,
    goals: [Vec3; 2],
    speed: f32,
    plan_fwd: Vec3,
    plan_speed: f32,
    head_on: bool,
}

/// The searching cop's brain state 7, STATE_SEARCH (enter F-57507, tick F-15801; docs/reference/hookmen/search_and_connections.md 2):
/// up to 10 points with the road or junction each lies on, driven by the racers' brain.
#[derive(Clone, Debug, Default)]
struct CopSearch {
    uid: u32,
    pts: Vec<(Vec3, Option<NavNode>)>,
    /// +340: the route is set
    routed: bool,
    /// +604: the cop has passed where the target was last seen (heads for point 1), has been near a goal over
    /// 5 s, or reached a one-point route; the search clock runs from then
    started: bool,
    /// +605: the list came out empty
    no_goals: bool,
    /// +580: seconds within 8 m of the goal it heads for (F-74140)
    near_t: f32,
    /// the state's time
    t: f32,
    /// +736 search from the cop's own place and turn at random at every junction; +740 the turn at the first
    /// junction (radians) - the pursuit's lone searcher: neither (F-72765)
    random: bool,
    angle: f32,
}

/// The search's points (F-52110): from `start` along the target's last known velocity `v` (at least 22.35 m/s)
/// x fMinSetGoalTime `per` until a point lies on a road or junction (within 3 m), then along the connection list
/// to the neighbour best in line with that direction, each next point the middle of the node's lane; 10 at most.
fn search_points(nav: &NavSys, world: &World, start: Vec3, v: Vec3, per: f32, first: bool, random: bool, angle: f32, rnd: &mut dyn FnMut() -> f32) -> Vec<(Vec3, Option<NavNode>)> {
    let mesh = crate::offroad::mesh();
    // F-58319: over the mesh, or F-45363's polygon within 10 m - which moves the point to its centre
    let nav_ok = |p: Vec3| -> Option<Vec3> {
        match mesh {
            None => Some(p),
            Some(m) if m.on_mesh(p) => Some(p),
            Some(m) => m.teleport_target(p),
        }
    };
    let probe = |p: Vec3, h: f32| -> Option<Vec3> { world.collider.raycast(p + Vec3::Y * h, -Vec3::Y, 2.0 * h).map(|hit| hit.point) };
    let v = if v.length_squared() < 499.523 { (if v.length_squared() <= 1e-5 { Vec3::X } else { v.normalize() }) * 22.35 } else { v };
    let s = v * per;
    let mut pts: Vec<(Vec3, Option<NavNode>)> = Vec::new();
    let mut p = start;
    let mut node = None;
    // phase 1: dead reckoning along S
    for _ in 0..10 {
        let mut ok = nav_ok(p);
        if ok.is_none() {
            if let Some(h) = probe(p, 15.0) {
                p = h;
                ok = nav_ok(p);
            }
        }
        if let Some(q) = ok {
            p = q;
        }
        if let Some(&(last, _)) = pts.last() {
            let mut k = 0;
            while k < 20 {
                if ok.is_some() && (p - last).length_squared() >= 1.0 {
                    break;
                }
                p += s;
                ok = nav_ok(p);
                if let Some(q) = ok {
                    p = q;
                }
                k += 1;
            }
            if k == 20 {
                if let Some(h) = crate::vehicles::traffic::snap_to_lane(nav, p, 50.0).0 {
                    p = h.point;
                    ok = nav_ok(p);
                    if let Some(q) = ok {
                        p = q;
                    }
                }
            }
        }
        if ok.is_some() {
            node = nav.node_within(p, 3.0, false).or_else(|| nav.node_within(p, 3.0, true));
            pts.push((p, node));
        }
        p += s;
        if node.is_some() {
            break;
        }
    }
    // phase 2: the connection list
    if !pts.is_empty() && pts.len() < 10 && pts.last().is_some_and(|q| q.1.is_some()) {
        let mut dir = glam::Vec2::new(s.x, s.z);
        dir = if dir.length_squared() < 1e-12 { glam::Vec2::ZERO } else { dir.normalize() };
        let mut turn = first || random;
        loop {
            let Some(r) = pts.last().and_then(|q| q.1) else { break };
            let rc = nav.node_rect(r).c;
            let mut best = -999.0f32;
            let mut n = None;
            for &x in nav.next_of(r) {
                if pts.iter().any(|q| q.1 == Some(x)) {
                    continue;
                }
                let xc = nav.node_rect(x).c;
                let d = glam::Vec2::new(xc[0] - rc[0], xc[1] - rc[1]);
                if d == glam::Vec2::ZERO {
                    continue;
                }
                let sc = d.normalize().dot(dir);
                if sc > best {
                    best = sc;
                    n = Some(x);
                }
            }
            let Some(n) = n else { break };
            if matches!(n, NavNode::Junction(_)) {
                if turn {
                    let (sn, cs) = angle.sin_cos();
                    let t = glam::Vec2::new(dir.x * cs + dir.y * sn, dir.y * cs - dir.x * sn);
                    dir = if t.length_squared() < 1e-12 { glam::Vec2::ZERO } else { t.normalize() };
                }
                turn = random && rnd() < 0.5;
            }
            // (F-59248: the node's lane at its middle; F-90092 on a road, else F-53622 - the road polygon within 10 m,
            // which moves it there -, else nav_ok)
            let Some(lane) = nav.node_rect(n).lane else { break };
            let Some(q) = nav.curves.get(lane).map(|c| c.eval(0.5).0) else { break };
            let q = match mesh {
                None => Some(q),
                Some(m) if m.on_road(q) => Some(q),
                Some(m) => m.road_teleport_target(q).or_else(|| nav_ok(q)),
            };
            let Some(q) = q else { break };
            pts.push((q, Some(n)));
            if pts.len() >= 10 {
                break;
            }
        }
    }
    pts
}

/// F-94676: the cop sees the target - within `r` (3D), within `half` degrees of its forward when `half` > 0, and a
/// clear line from 2 m above the target to 2 m above the cop (F-69576).
fn cop_sees(world: &World, cop: &crate::vehicles::CarInstance, tpos: Vec3, r: f32, half: f32) -> bool {
    let to = tpos - cop.state.pos;
    if r > 0.0 && to.length_squared() > r * r {
        return false;
    }
    if half > 0.0 && cop.state.forward().angle_between(to) >= half.to_radians() {
        return false;
    }
    let (a, b) = (tpos + Vec3::Y * 2.0, cop.state.pos + Vec3::Y * 2.0);
    let d = a.distance(b);
    d < 1e-4 || world.collider.raycast(a, (b - a) / d, d).is_none()
}

/// The police manager's tactic kinds (types 0 - 4; docs/reference/hookmen/clip_time_and_tactics.md B): how many cops
/// each takes (+72, set by the ctors) and the floor StartTactic (F-33550) raises the law score to. A higher kind
/// replaces a lower one on the same target.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum TacKind {
    #[default]
    None,
    Dispatch,
    PullOver,
    Pursuit,
    PursuitDbl,
    ArrowTrap,
}

impl TacKind {
    fn cops(self) -> usize {
        match self {
            TacKind::None => 0,
            TacKind::PursuitDbl => 2,
            TacKind::ArrowTrap => 3,
            _ => 1,
        }
    }

    fn floor(self) -> f32 {
        match self {
            TacKind::PullOver => 250.0,
            TacKind::Pursuit | TacKind::PursuitDbl => 600.0,
            TacKind::ArrowTrap => 1000.0,
            _ => 0.0,
        }
    }

    /// the Double Pursuit's and the Arrow Trap's squad FSM (formation, swipes) instead of the Pursuit's
    fn squad(self) -> bool {
        matches!(self, TacKind::PursuitDbl | TacKind::ArrowTrap)
    }
}

/// The squad tactics' state (the Arrow Trap F-76774, the Double Pursuit F-40655; clip_time_and_tactics.md B.1 / B.2): CatchUp (the
/// Double Pursuit's AlongSide) with its formation, or the Arrow Trap's Halt; Search and Arrest are the port's heat.
#[derive(Clone, Debug, Default)]
struct Squad {
    /// CatchUp (the Double Pursuit's AlongSide) entered
    begun: bool,
    halt: bool,
    /// +16: the formation's lock, +20: its hysteresis
    lock: f32,
    hyst: f32,
    /// +24: the right flank's wait before its swipe, +32: its window latch
    swipe_t: f32,
    latch: bool,
    /// +164 the right flank (cop A), +168 the left flank (cop B), +172 the trailing cop (C)
    slots: [Option<u32>; 3],
}

/// One cop's brain visibility (+596 unseen time, +600 the time in the search cone; F-54688).
#[derive(Clone, Copy, Debug, Default)]
struct Eye {
    unseen: f32,
    cone_t: f32,
}

/// Dispatch to Location (type 0, update F-87106; clip_time_and_tactics.md B.3): 0 WaitToDispatch, 1 Dispatch, 2 Respond,
/// 3 WaitAtLocation.
#[derive(Clone, Debug)]
struct Dispatch {
    cop: u32,
    st: u8,
    t: f32,
    /// +176: where the player was when it started
    loc: Vec3,
    /// the trigger's DISPATCH_* call
    call: String,
    /// +208: the player's trail, a point a second (15 at most), +452 its timer, +456 the seen timer
    crumbs: Vec<Vec3>,
    crumb_t: f32,
    seen_t: f32,
    /// WaitAtLocation drives the trail (true) or searches
    trail: bool,
    /// the brain's route is set; it has arrived (brain +576 == 1)
    routed: bool,
    arrived: bool,
}

/// The race's cop zone (PoliceManager +188 zone, +192 lap, +196 state, +212 timer).
#[derive(Clone, Debug)]
pub struct CopZone {
    pub idx: usize,
    pub lap: usize,
    /// 0 waiting for its lap, 1 armed, 2 set off
    pub state: u8,
    pub timer: f32,
    /// the player set it off
    pub player: bool,
    /// a cruiser waits there (the port's: where the original's unit comes from was not read)
    pub stationed: bool,
}

/// One of the three cruisers the hookmen's scripts make (`create_units`).
#[derive(Clone, Debug)]
pub struct Unit {
    /// 400 + i
    pub id: u32,
    /// where it was made and comes up (+912), and its heading (degrees, the script's)
    pub home: Vec3,
    pub heading: f32,
    /// its underground slot (+896)
    pub slot: Vec3,
    /// underground (+880)
    pub under: bool,
}

/// The three of `red_light_hk1` 0x436e / `freeway_hookman<n>` 0x2c73: (home, heading, slot).
pub const SCRIPT_UNITS: [([f32; 3], f32, [f32; 3]); 3] = [
    ([2030.1, 6.8, -813.7], 90.0, [-510.0, 9.5, -400.0]),
    ([-646.8, 18.5, -643.8], 90.0, [-500.0, 9.5, -400.0]),
    ([-2434.4, 1.9, -577.9], 84.9, [-490.0, 9.5, -400.0]),
];

pub struct Police {
    /// the scripted cruisers 400..402 (`create_units`)
    pub units: Vec<Unit>,
    pub tun: Tuning,
    paths: Vec<Vec<Vec3>>,
    pub heat: Heat,
    /// law-breaking score 0..iMaxLawBreakingScore
    pub score: f32,
    pub fine: f32,
    /// distinct laws broken since the score was last clear (bit mask)
    offences: u32,
    /// what the last ticket was for: 1 reckless driving, 2 traffic violation, 4 property
    pub ticket_for: u32,
    lost: f32,
    seen_time: f32,
    /// the search clock (the Search state's +16, F-40917): runs only once the searcher has started
    pub search: f32,
    /// the tactic on the player and its cops (+76; cop 0 the request cop, Engage F-11097): only they chase
    pub kind: TacKind,
    cops: Vec<u32>,
    /// +28: may escalate (1 for the law ladder's tactics, 0 for the payback's goons)
    may_escalate: bool,
    sq: Squad,
    /// each cop's brain visibility
    eyes: HashMap<u32, Eye>,
    /// the dispatch to the player's place (a radio dispatch trigger)
    dsp: Option<Dispatch>,
    /// a DISPATCH trigger fired (main: the city's global trigger scripts): its call
    pub dispatch_call: Option<String>,
    /// the police radio is speaking (main), the radio byte F-62213 reads; a race is running (F-42951: a police cop in a
    /// race plans the plain way and never swipes)
    pub radio_busy: bool,
    pub in_race: bool,
    /// +136: while above 0 the brains' lost flag +773 is off (F-79519; F-63286 counts it down and sets the flag
    /// again); the tactic's begin (F-82656) sets fTrackTgtTime - the manager's track byte is 1 by default (F-39756)
    track_t: f32,
    /// the lead's brain: unseen time +596, the time in the search cone +600, the last known velocity +624
    last_known_v: Vec3,
    /// the searchers' brain state 7
    srch: Vec<CopSearch>,
    prev_heat: Heat,
    /// a line for the police radio from the search (F-40917): REPORT_SUSPECT_LOST when the clock starts, _SPOTTED or
    /// _REACQUIRE when it ends with a sighting after 2 s
    pub radio_cue: Vec<String>,
    arrest: f32,
    /// the arrest safe zones (`load_safe_zones`)
    pub safe_zones: Vec<(Vec3, f32)>,
    /// the arrest's COP_ARREST is posted (its scene may hold the arrest on, `hold_busted`)
    arrest_posted: bool,
    /// the tactic's seen / unseen timers (+124 / +132, F-74492) and the seen timer of the frame before (+128)
    t_seen: f32,
    t_seen_prev: f32,
    t_unseen: f32,
    busted_timer: f32,
    /// the cops' arrest is over (its hold run out): a race running ends back in the cruise (F-22530)
    pub arrest_over: bool,
    /// the race's cop zone (F-88125 picks it, F-71212: the zone machine) - None outside a race
    pub zone: Option<CopZone>,
    /// the zone's chase is due: the pursuit starts on the next update
    zone_go: bool,
    /// the zone of the race running was picked
    pub zone_picked: bool,
    /// the pull-over's bullhorn (its tactic +92 = 2 at iBullhornLawBreakScore, F-22530): seconds left of
    /// fBullhornTime; Some(0) = run out (the 599 cap is lifted)
    bullhorn: Option<f32>,
    /// the ticket hold (FsmPullOver StateTicket, F-25878 / F-46174): seconds left before COP_PULLOVER
    ticket_hold: Option<f32>,
    /// COP_PULLOVER is running (CopPullover.sco): the police wait for `ticket_done` or `suspect_fled`
    ticketing: bool,
    /// after a tactic on the player ends: no scoring for 5 s (the record's +116, F-22530 / F-54211)
    grace: f32,
    /// the pursuit after a bolt from the ticket starts with its cruisers held this long (the Pursuit
    /// tactic's Delay state, F-10824 / F-50798: request +20 = the bolt's time x fPullOverFleeDelayScale)
    flee_hold: f32,
    /// each pursuing cruiser's tactic state (by car uid), and the target's speed at the last update (tactic +64)
    tacs: HashMap<u32, CopTac>,
    last_tspeed: f32,
    /// the highest speed band whose start points were paid (the record's "started" bits +100: kept to `clear`)
    speed_level: u8,
    /// the highest speed band of the offences so far (1..3; 0 = none): the fine counts that one
    speed_top: u8,
    /// the seconds counters of the per-second points (F-52240: they pay whole seconds): 0 speed,
    /// 1 wrong way, 2 burn-out, 3 two-wheel driving, 4 off-road, 5 the fall below iPullOverLawBreakScore,
    /// 6 the climb from it
    sec: [f32; 7],
    /// the player is off the roads, since the start points were paid
    off_active: bool,
    /// two-wheel driving has been going on since its start points were paid
    two_active: bool,
    /// the jump in the air has been paid (the original's byte +140)
    jump_paid: bool,
    /// the pull-over was run from (the fine's `iPullOverFleeFine`)
    fled: bool,
    /// the share of the career (the Progression system's fraction, 0..1) that cuts the fine;
    /// set by the game each frame
    pub progress: f32,
    wrong_way: f32,
    wrong_active: bool,
    burnout: f32,
    burnout_active: bool,
    nav_check: f32,
    against_traffic: bool,
    last_knocked: u32,
    last_damage: f32,
    spawn_timer: f32,
    /// how long the running pursuit has lasted (seconds)
    pub chase_time: f32,
    last_known: Vec3,
    /// time until the next radio report of the target's position
    radio: f32,
    time: f32,
    rng: u32,
    glow_buf: Option<wgpu::Buffer>,
    glow_count: u32,
    pub enabled: bool,
    /// junction the player was last seen approaching on red
    red_junction: Option<usize>,
    /// step 19d: the pursuit under way is the payback client's friends' (`start_goons`):
    /// their cars (`CarInstance::goon`) are the pursuers, no cruiser is sent, nothing is
    /// lit, and the two results are the payback leg's to deal with
    pub goons: bool,
}

const LAW_SPEED: u32 = 1;
const LAW_WRONG_WAY: u32 = 2;
const LAW_BURNOUT: u32 = 4;
const LAW_PROP: u32 = 8;
const LAW_VEHICLE: u32 = 16;
const LAW_POLICE_VEHICLE: u32 = 32;
const LAW_RED_LIGHT: u32 = 64;
const LAW_ROOF_JUMP: u32 = 128;
const LAW_TWO_WHEEL: u32 = 256;
const LAW_OFF_ROAD: u32 = 512;
const MAX_GLOWS: usize = 16;

impl Police {
    pub fn new() -> Police {
        Police {
            tun: Tuning::default(),
            paths: Vec::new(),
            heat: Heat::None,
            score: 0.0,
            fine: 0.0,
            offences: 0,
            ticket_for: 0,
            lost: 0.0,
            seen_time: 0.0,
            search: 0.0,
            arrest: 0.0,
            safe_zones: Vec::new(),
            arrest_posted: false,
            t_seen: 0.0,
            t_seen_prev: 0.0,
            t_unseen: 0.0,
            busted_timer: 0.0,
            kind: TacKind::None,
            cops: Vec::new(),
            may_escalate: false,
            sq: Squad::default(),
            eyes: HashMap::new(),
            dsp: None,
            dispatch_call: None,
            radio_busy: false,
            in_race: false,
            track_t: 0.0,
            last_known_v: Vec3::ZERO,
            srch: Vec::new(),
            prev_heat: Heat::None,
            radio_cue: Vec::new(),
            arrest_over: false,
            zone: None,
            zone_go: false,
            zone_picked: false,
            tacs: HashMap::new(),
            last_tspeed: 0.0,
            bullhorn: None,
            ticket_hold: None,
            ticketing: false,
            grace: 0.0,
            flee_hold: 0.0,
            speed_level: 0,
            speed_top: 0,
            sec: [0.0; 7],
            off_active: false,
            two_active: false,
            jump_paid: false,
            fled: false,
            progress: 1.0,
            wrong_way: 0.0,
            wrong_active: false,
            burnout: 0.0,
            burnout_active: false,
            nav_check: 0.0,
            against_traffic: false,
            last_knocked: 0,
            last_damage: 0.0,
            spawn_timer: 8.0,
            chase_time: 0.0,
            last_known: Vec3::ZERO,
            radio: 0.0,
            time: 0.0,
            rng: 0x51ed_270b,
            glow_buf: None,
            glow_count: 0,
            enabled: true,
            red_junction: None,
            goons: false,
            units: Vec::new(),
        }
    }

    pub fn load(&mut self, data_dir: &str) {
        let dir = Path::new(data_dir).join("tune/police");
        self.tun = Tuning::load(&dir.join("globals.xml"));
        self.safe_zones = load_safe_zones(Path::new(data_dir));
        self.paths = load_patrol_paths(&dir.join("patrolpath.xml"));
        log::info!("police: {} tuning values, {} patrol paths", self.tun.v.len(), self.paths.len());
    }

    fn rand(&mut self) -> f32 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        (x % 100000) as f32 / 100000.0
    }

    /// PoliceManager_CreatePolice for the three units (only while the police are unlocked -
    /// F-17632): made at their home, sent underground at once. A unit of the same id is
    /// made again.
    pub fn create_units(&mut self, unlocked: bool) {
        if !unlocked {
            return;
        }
        for (i, (home, heading, slot)) in SCRIPT_UNITS.iter().enumerate() {
            let id = 400 + i as u32;
            self.units.retain(|u| u.id != id);
            self.units.push(Unit { id, home: Vec3::from(*home), heading: *heading, slot: Vec3::from(*slot), under: true });
        }
        log::info!("police: units 400..402 made, underground");
    }

    /// PoliceManager_DestroyAllPolice: every cruiser and every unit.
    pub fn destroy_all(&mut self, vehicles: &mut Vehicles) {
        self.reset(vehicles);
        if !self.units.is_empty() {
            log::info!("police: all destroyed ({} units)", self.units.len());
        }
        self.units.clear();
    }

    /// PoliceManager_SetAllPoliceUnderground(1, 1): the units go underground (no new car is put
    /// up for them; a unit's car already on the road is left where it is - approx.).
    pub fn units_down(&mut self, vehicles: &mut Vehicles) {
        for u in self.units.iter_mut() {
            u.under = true;
        }
        // (an underground unit has no car on the road, and so none for IsAnyPoliceCloseToPlayer,
        // F-91297, which skips the police whose underground flag +880 is set)
        vehicles.cars.retain(|c| !(c.cop && c.vinyl_name.starts_with("police_unit:")));
    }

    /// PoliceManager_SetAllPoliceUnderground(0, 1): the units come up at their surface place;
    /// the port puts each there (`update_units`) while the player is within streaming reach.
    pub fn units_up(&mut self) {
        for u in self.units.iter_mut() {
            u.under = false;
        }
    }

    /// The units that are up stand at their home as the script made them (heading in degrees -
    /// the port's yaw is the heading in radians) while the player is within 380 m; a unit whose
    /// car has gone counts as underground again.
    pub fn update_units(&mut self, gpu: &Gpu, world: &World, vehicles: &mut Vehicles) {
        let Some(pp) = vehicles.player().map(|c| c.state.pos) else { return };
        let Some(model) = vehicles.models.iter().find(|m| m.name == COP_CAR).cloned() else { return };
        for u in self.units.iter_mut() {
            let tag = format!("police_unit:{}", u.id);
            let has = vehicles.cars.iter().any(|c| c.cop && c.vinyl_name == tag);
            if u.under || has || u.home.distance(pp) > 380.0 {
                continue;
            }
            if vehicles.cars.iter().any(|c| c.state.pos.distance(u.home) < 6.0) {
                continue;
            }
            let idx = vehicles.spawn(gpu, model.clone(), u.home + Vec3::Y * 0.8, u.heading.to_radians(), [0.92, 0.92, 0.92, 0.0], false);
            Self::dress_cruiser(world, vehicles, idx);
            let c = &mut vehicles.cars[idx];
            c.cop = true;
            c.vinyl_name = tag;
            log::info!("police: unit {} up at its home ({:.0} m from the player)", u.id, u.home.distance(pp));
        }
    }

    /// Forget everything (new session / race start).
    pub fn reset(&mut self, vehicles: &mut Vehicles) {
        self.heat = Heat::None;
        self.score = 0.0;
        self.fine = 0.0;
        self.offences = 0;
        self.speed_level = 0;
        self.lost = 0.0;
        self.search = 0.0;
        self.arrest = 0.0;
        self.spawn_timer = 8.0;
        vehicles.cars.retain(|c| !c.cop);
    }

    /// Keep the arrest going for this long (its cutscene is playing).
    pub fn hold_busted(&mut self, secs: f32) {
        if self.heat == Heat::Busted {
            self.busted_timer = self.busted_timer.max(secs);
        }
    }

    /// PoliceManager_EvaluateCopZones (F-88125): of the race's zones ONE is picked, and ONE lap
    /// (`rand % laps` when the race has more than one, else lap 0); lap 0's zone is armed at once,
    /// another's when the race's lap changes to it (F-82127, "[COP ZONE] lap [%d], cop zone enabled").
    /// (approx.: the port's own random numbers, not the game's LCG)
    pub fn pick_zone(&mut self, zones: usize, laps: usize) {
        self.zone_go = false;
        self.zone = (zones > 0).then(|| {
            let idx = (self.rand() * zones as f32) as usize % zones;
            let lap = if laps > 1 { (self.rand() * laps as f32) as usize % laps } else { 0 };
            log::info!("police: cop zone {} enabled on lap {}", idx, lap);
            CopZone { idx, lap, state: if lap == 0 { 1 } else { 0 }, timer: 0.0, player: false, stationed: false }
        });
        if zones == 0 {
            log::info!("police: no cops in this race");
        }
    }

    /// The zone machine of a race (F-71212), per frame: `lap` is the race's current lap, `inside`
    /// whether a racer is in the zone (and whether it is the player). State 1 (armed): the first
    /// racer in the zone sets it off (state 2, 5.5 s). State 2: when the time is up the pursuit
    /// starts. (approx./not ported: the pursuit on an AI racer - the original's target is the racer
    /// that set it off, or with iRaceTgtAiProbPrcnt the nearest AI racer; the port's police chase
    /// only the player, so a zone set off for an AI racer starts nothing; the earlier start when the
    /// target comes in range (F-12032) and the state 3 wait after it)
    pub fn zone_step(&mut self, lap: usize, inside: Option<bool>, dt: f32) -> Option<&'static str> {
        let Some(z) = self.zone.as_mut() else { return None };
        let mut say = None;
        match z.state {
            0 if lap == z.lap => {
                z.state = 1;
                log::info!("police: lap {}, cop zone enabled", lap);
            }
            1 => {
                if let Some(player) = inside {
                    z.state = 2;
                    z.timer = 5.5;
                    z.player = player;
                    log::info!("police: cop zone {} set off by {}", z.idx, if player { "the player" } else { "an AI racer" });
                    say = Some("PLAYER_COPS_BEGIN_PURSUIT");
                }
            }
            2 => {
                z.timer -= dt;
                if z.timer < 0.0 {
                    z.state = 0;
                    z.lap = usize::MAX;
                    if z.player && matches!(self.heat, Heat::None | Heat::PullOver) {
                        self.zone_go = true;
                    }
                }
            }
            _ => {}
        }
        say
    }

    /// Test helper: start a pursuit right now with a cruiser close by.
    pub fn debug_force(&mut self, gpu: &Gpu, world: &World, vehicles: &mut Vehicles) {
        self.score = 700.0;
        // (started as a cop zone's chase is: no cop has to see the car first)
        self.zone_go = true;
        if let Some((pos, fwd)) = vehicles.player().map(|c| (c.state.pos, c.state.forward())) {
            let ok = self.spawn_cop(gpu, world, vehicles, pos, fwd, 18.0, 80.0, None);
            log::info!("police: test cruiser spawned: {}", ok);
        }
    }

    /// CopPullover.sco is over with the ticket: the police record is cleared (ResetAllPolice / the tactic's end)
    /// and nothing is scored for 5 s.
    pub fn ticket_done(&mut self) {
        self.clear();
        self.grace = 5.0;
    }

    /// The suspect bolted from the ticket scene `secs` into its walk (PoliceManager_SuspectFleeing, F-72120):
    /// the run is on the fine (`iPullOverFleeFine`) and a pursuit starts whose cruisers wait
    /// secs x fPullOverFleeDelayScale first.
    pub fn suspect_fled(&mut self, secs: f32, at: Vec3) {
        self.ticketing = false;
        self.heat = Heat::Pursuit;
        self.fled = true;
        self.score = self.score.max(self.tun.get("iPursuitLawBreakScore", 600.0));
        self.flee_hold = secs * self.tun.get("fPullOverFleeDelayScale", 2.0);
        self.last_known = at;
        self.fine = self.law_fines();
        self.lost = 0.0;
        self.arrest = 0.0;
        let (cops, tt) = (self.cops.clone(), self.tun.get("fTrackTgtTime", 25.0));
        self.start_tactic(TacKind::Pursuit, cops, true, tt);
    }

    /// Engage (F-11097): the tactic's `n` cops - `first` (the request cop), then the manager's cops of the group nearest
    /// the target (squared 3D distance, any distance) not taken yet. The manager's units that have no car on the road
    /// are streamed in for it (the original waits for them, +84; the port puts one on a road 130 - 260 m away -
    /// where the original's comes up was not read). None when a slot stays empty.
    fn engage(&mut self, gpu: &Gpu, world: &World, vehicles: &mut Vehicles, n: usize, first: Option<u32>, at: Vec3, fwd: Vec3) -> Option<Vec<u32>> {
        let mut out: Vec<u32> = first.into_iter().collect();
        let goons = self.goons;
        let mut cands: Vec<(f32, u32)> = vehicles.cars.iter().filter(|c| if goons { c.goon } else { c.cop && !c.goon }).filter(|c| !out.contains(&c.uid)).map(|c| (c.state.pos.distance_squared(at), c.uid)).collect();
        cands.sort_by(|a, b| a.0.total_cmp(&b.0));
        for (_, u) in cands {
            if out.len() >= n {
                break;
            }
            out.push(u);
        }
        while out.len() < n && !goons {
            let Some(ui) = self.units.iter().position(|u| !vehicles.cars.iter().any(|c| c.cop && c.vinyl_name == format!("police_unit:{}", u.id))) else { break };
            if !self.spawn_cop(gpu, world, vehicles, at, fwd, 130.0, 260.0, None) {
                break;
            }
            let id = self.units[ui].id;
            self.units[ui].under = false;
            let Some(c) = vehicles.cars.last_mut().filter(|c| c.cop) else { break };
            c.vinyl_name = format!("police_unit:{}", id);
            log::info!("police: unit {} streamed in for the tactic", id);
            out.push(c.uid);
        }
        (out.len() >= n).then_some(out)
    }

    /// The dispatch cop's search (cmd 7, +736 = 1: from its own place, a random turn at each junction, the first
    /// +-pi/2 at random; F-52732 / F-92001). The direction: the target's velocity now (the brain's last known
    /// velocity of a dispatch was not read).
    fn dispatch_search(&mut self, world: &World, vehicles: &Vehicles, uid: u32, per: f32) {
        let (Some(c), Some(nav)) = (vehicles.cars.iter().find(|c| c.uid == uid), world.nav.as_ref()) else { return };
        let v = vehicles.player().map_or(Vec3::ZERO, |p| p.state.vel);
        self.last_known_v = v;
        let angle = if crate::brain::rng().float() < 0.5 { std::f32::consts::FRAC_PI_2 } else { -std::f32::consts::FRAC_PI_2 };
        let mut rnd = || crate::brain::rng().float();
        let pts = search_points(nav, world, c.state.pos, v, per, true, true, angle, &mut rnd);
        log::info!("police: dispatch cruiser {} searches: {} goals", uid, pts.len());
        self.srch.retain(|s| s.uid != uid);
        self.srch.push(CopSearch { uid, no_goals: pts.is_empty(), pts, random: true, angle, ..Default::default() });
    }

    /// StartTactic (F-33550): the old tactic ends with flag 0 (its cops lose their commands; lights kept), the score
    /// goes to the kind's floor; the base begin (F-82656): +136 = fTrackTgtTime (the manager's track byte, 1 by default -
    /// F-39756) for the kinds whose slot 7 reads it, -1 (never lost) for the pull-over.
    fn start_tactic(&mut self, kind: TacKind, cops: Vec<u32>, may_escalate: bool, track: f32) {
        log::info!("police: tactic {:?} -> {:?} with cruisers {:?}", self.kind, kind, cops);
        self.kind = kind;
        self.cops = cops;
        self.may_escalate = may_escalate;
        self.tacs.clear();
        self.srch.clear();
        self.eyes.clear();
        self.sq = Squad { hyst: 2.0, lock: 0.0, ..Default::default() };
        self.score = self.score.max(kind.floor());
        self.track_t = if kind == TacKind::PullOver { -1.0 } else { track };
        if kind != TacKind::Dispatch {
            self.dsp = None;
        }
    }

    /// The number of cops in the latest tactic (police manager +80, PoliceManager_GetNumCopsInLatestTactic).
    pub fn tactic_cops(&self) -> usize {
        self.cops.len()
    }

    pub fn lights_on(&self) -> bool {
        !matches!(self.heat, Heat::None) && !self.goons
    }

    /// Step 19d: one of the payback client's friends comes onto a road near the player (the
    /// original keeps the three "underground" and streams them in for the pursuit - where,
    /// was not read: the port puts them where it puts a cruiser sent to a pursuit).
    pub fn spawn_goon(&mut self, gpu: &Gpu, world: &World, vehicles: &mut Vehicles, model: std::sync::Arc<crate::vehicles::CarModel>, color: [f32; 4], set_up: &str, driver: &str) -> Option<usize> {
        let (center, heading) = vehicles.player().map(|c| (c.state.pos, c.state.forward()))?;
        let nav = world.nav.as_ref()?;
        let cands: Vec<usize> = nav
            .links_near(center, 260.0)
            .filter(|&i| {
                let l = &nav.links[i];
                let d = l.a.distance(center);
                !l.next.is_empty() && !l.prev.is_empty() && d > 110.0 && d < 260.0 && l.len > 12.0
            })
            .collect();
        if cands.is_empty() {
            return None;
        }
        let mut best = None;
        let mut bs = f32::NEG_INFINITY;
        for _ in 0..8 {
            let c = cands[(self.rand() * cands.len() as f32) as usize % cands.len()];
            let l = &nav.links[c];
            let pos = l.point_at(0.5) + Vec3::Y * 0.8;
            if !world.ready_at(pos) || vehicles.cars.iter().any(|v| v.state.pos.distance(pos) < 14.0) {
                continue;
            }
            // (from behind, as pursuers come)
            let s = -(l.a - center).normalize_or(Vec3::X).dot(heading) + self.rand() * 0.5;
            if s > bs {
                bs = s;
                best = Some(c);
            }
        }
        let best = best?;
        let l = &nav.links[best];
        let pos = l.point_at(0.5) + Vec3::Y * 0.8;
        let dir = l.dir();
        let idx = vehicles.spawn(gpu, model, pos, (-dir.x).atan2(-dir.z), color, false);
        let c = &mut vehicles.cars[idx];
        c.goon = true;
        c.pinned = true;
        c.vinyl_name = set_up.to_string();
        c.driver_set = driver.to_string();
        if let Some(ai) = c.ai.as_mut() {
            ai.link = Some(best);
            ai.t = 0.5;
            ai.target_speed = 14.0;
        }
        log::info!("police: a friend of the client ({}) comes at {:.0} m", set_up, pos.distance(center));
        Some(idx)
    }

    /// Step 19d: PoliceManager_StartPursuit(player, 3, 0, 0, 0) for the payback's chasers:
    /// the pursuit tactic on the player with the cars made for it. The law-breaking score
    /// goes to its maximum (F-33550 A-46466: at least the table's first entry).
    pub fn start_goons(&mut self, vehicles: &mut Vehicles) {
        // (approx.: the cruisers on the road are taken off for the chase - whether the
        // original's tactic can pick one of them beside the three was not read)
        vehicles.cars.retain(|c| !c.cop);
        self.goons = true;
        self.heat = Heat::Pursuit;
        // (PoliceManager_StartPursuit kind 3, F-92929: an Arrow Trap of the group-0 cars, may-escalate byte 0)
        let (cops, tt) = (vehicles.cars.iter().filter(|c| c.goon).map(|c| c.uid).collect(), self.tun.get("fTrackTgtTime", 25.0));
        self.start_tactic(TacKind::ArrowTrap, cops, false, tt);
        self.score = self.tun.get("iMaxLawBreakingScore", 1000.0);
        self.fine = 0.0;
        self.lost = 0.0;
        self.search = 0.0;
        self.arrest = 0.0;
        self.chase_time = 0.0;
        self.last_known = vehicles.player().map_or(Vec3::ZERO, |c| c.state.pos);
    }

    /// The payback's chase is over (PoliceManager_DestroyPolice for the three once
    /// `Shutdown_PaybackOpponents` is set): the cars go, the police are as before it.
    pub fn end_goons(&mut self, vehicles: &mut Vehicles) {
        vehicles.cars.retain(|c| !c.goon);
        if self.goons {
            self.goons = false;
            self.clear();
        }
    }

    /// Law-breaking score as 0..1.
    pub fn level(&self) -> f32 {
        (self.score / self.tun.get("iMaxLawBreakingScore", 1000.0)).clamp(0.0, 1.0)
    }

    fn spawn_cop(&mut self, gpu: &Gpu, world: &World, vehicles: &mut Vehicles, center: Vec3, heading: Vec3, min_d: f32, max_d: f32, near: Option<Vec3>) -> bool {
        let Some(model) = vehicles.models.iter().find(|m| m.name == COP_CAR).cloned() else { return false };
        let Some(nav) = world.nav.as_ref() else { return false };
        let around = near.unwrap_or(center);
        let radius = if near.is_some() { 60.0 } else { max_d };
        let cands: Vec<usize> = nav
            .links_near(around, radius)
            .filter(|&i| {
                let l = &nav.links[i];
                let d = l.a.distance(center);
                !l.next.is_empty() && !l.prev.is_empty() && d > min_d && d < max_d && l.len > 12.0
            })
            .collect();
        if cands.is_empty() {
            return false;
        }
        // during a pursuit cruisers prefer to come from ahead
        let mut best = cands[(self.rand() * cands.len() as f32) as usize % cands.len()];
        if self.heat == Heat::Pursuit {
            let mut bs = f32::NEG_INFINITY;
            for _ in 0..6 {
                let c = cands[(self.rand() * cands.len() as f32) as usize % cands.len()];
                let s = (nav.links[c].a - center).normalize_or(Vec3::X).dot(heading) + self.rand() * 0.5;
                if s > bs {
                    bs = s;
                    best = c;
                }
            }
        }
        let l = &nav.links[best];
        let pos = l.point_at(0.5) + Vec3::Y * 0.8;
        if !world.ready_at(pos) || vehicles.cars.iter().any(|c| c.state.pos.distance(pos) < 14.0) {
            return false;
        }
        let dir = l.dir();
        let idx = vehicles.spawn(gpu, model, pos, (-dir.x).atan2(-dir.z), [0.92, 0.92, 0.92, 0.0], false);
        Self::dress_cruiser(world, vehicles, idx);
        let c = &mut vehicles.cars[idx];
        c.cop = true;
        if let Some(ai) = c.ai.as_mut() {
            ai.link = Some(best);
            ai.t = 0.5;
            ai.target_speed = 14.0;
        }
        log::debug!("police: cruiser spawned at {:?}", pos);
        true
    }

    /// A cruiser wears its stock set-up's paint and plate (`resources/vehicle/configs/vp_chv_police_96.xcc`: black body,
    /// light grey where the model marks the sixth paint area - doors and roof -, grey metallic bumpers): black and white,
    /// as the original's own showroom picture of it (GitHub #17: the port painted it all white). This is the set-up the
    /// original hands every cruiser: its police manager loads this file once at start-up (F-46996 through F-67050), and
    /// the spawn (F-49535) copies that set-up (F-62644) when the car asked for is this one - which every script's
    /// CreatePolice call asks for - instead of loading one by name (F-51560).
    fn dress_cruiser(world: &World, vehicles: &mut Vehicles, idx: usize) {
        let Ok(b) = std::fs::read(world.assets.root.join("resources/vehicle/configs").join(format!("{COP_CAR}.xcc"))) else { return };
        let Some(s) = crate::mc4::setup_look(&b) else { return };
        let c = &mut vehicles.cars[idx];
        c.color = s.color;
        c.look = Some(std::sync::Arc::new(s.look));
        let plate = crate::mc4::setup_plate(&b);
        if !plate.is_empty() {
            c.plate_text = plate;
            c.plate = None;
        }
    }

    /// Put a cruiser in wait beside the road at `pos` (the cop zones of a race).
    pub fn station(&mut self, gpu: &Gpu, world: &World, vehicles: &mut Vehicles, pos: Vec3, slot: usize) -> bool {
        let Some(model) = vehicles.models.iter().find(|m| m.name == COP_CAR).cloned() else { return false };
        let Some(nav) = world.nav.as_ref() else { return false };
        let Some((li, t, d)) = nav.nearest_link(pos) else { return false };
        if d > 60.0 {
            return false;
        }
        let l = &nav.links[li];
        let dir = l.dir();
        let right = Vec3::new(-dir.z, 0.0, dir.x);
        // at the kerb if there is room, else in the lane
        let mut p = l.point_at(t) + Vec3::Y * 0.8;
        let from = p + Vec3::Y * 0.3;
        if world.collider.raycast(from, right, 5.2).is_none() {
            p += right * 3.4;
        }
        if !world.ready_at(p) || vehicles.cars.iter().any(|c| c.state.pos.distance(p) < 7.0) {
            return false;
        }
        let idx = vehicles.spawn(gpu, model, p, (-dir.x).atan2(-dir.z), [0.92, 0.92, 0.92, 0.0], false);
        Self::dress_cruiser(world, vehicles, idx);
        let c = &mut vehicles.cars[idx];
        c.cop = true;
        c.pinned = true;
        c.parked = Some(usize::MAX - 9000 - slot);
        c.settle = 1.5;
        if let Some(ai) = c.ai.as_mut() {
            ai.link = Some(li);
            ai.t = t;
            ai.target_speed = 14.0;
        }
        log::info!("police: cruiser in wait at ({:.0}, {:.0}, {:.0})", p.x, p.y, p.z);
        true
    }

    /// Advance the police simulation. `knocked` is the running count of props the cars
    /// have knocked over. Returns what happened this frame.
    pub fn update(&mut self, gpu: &Gpu, world: &World, vehicles: &mut Vehicles, knocked: u32, dt: f32) -> Option<Event> {
        match self.heat {
            Heat::Pursuit | Heat::Search => self.chase_time += dt,
            // (kept through the arrest for its form: taken by the arrest's scene)
            Heat::PullOver => self.chase_time = 0.0,
            Heat::None | Heat::Busted => {}
        }
        // each cruiser's siren target (F-78189, only for a cop made with lights - not the payback's goons): every
        // tick of a pull-over or pursuit tactic its cops get 1 (F-63286, the tactic's vt+52), a police scene sets
        // every cop to 1 each frame while it runs (F-54211) and its end every cop to 0 (F-30655); a tactic's end
        // turns its cops' lights off unless the target was arrested (F-67316). A cop in no tactic keeps its value.
        // (The port's search is the pursuit's lost-target part: lit.)
        if !self.goons {
            let lit = matches!(self.heat, Heat::Pursuit | Heat::PullOver | Heat::Search | Heat::Busted) || self.ticketing;
            let dsp_lit = self.dsp.as_ref().filter(|d| d.st >= 2).map(|d| d.cop);
            for c in vehicles.cars.iter_mut().filter(|c| c.cop) {
                if (lit && self.cops.contains(&c.uid)) || dsp_lit == Some(c.uid) {
                    c.siren = true;
                } else if self.heat == Heat::None && c.siren && self.dsp.as_ref().map_or(true, |d| d.cop != c.uid) {
                    c.siren = false;
                }
            }
        }
        if (!self.enabled && !self.goons) || dt <= 0.0 || self.ticketing {
            return None;
        }
        self.time += dt;
        // (step 19d: in the payback's chase the pursuers are the client's friends - all of them, as the original's
        // tactic waits for the three to be streamed in)
        let goons = self.goons;
        if goons {
            self.cops = vehicles.cars.iter().filter(|c| c.goon).map(|c| c.uid).collect();
        }
        // (only the tactic's cops chase; F-76774 result 3: one of them gone - the police are lost)
        let gone = self.kind > TacKind::Dispatch && !goons && self.cops.iter().any(|u| !vehicles.cars.iter().any(|c| c.uid == *u));
        let members = self.cops.clone();
        let pursuer = |c: &crate::vehicles::CarInstance| members.contains(&c.uid);
        // cruisers lying in wait join in once there is something to chase
        if !matches!(self.heat, Heat::None) {
            for c in vehicles.cars.iter_mut().filter(|c| c.cop && c.parked.is_some()) {
                c.parked = None;
                c.pinned = false;
            }
        }
        let Some(p) = vehicles.player() else { return None };
        let ppos = p.state.pos;
        let pvel = p.state.vel;
        let prev_heat = std::mem::replace(&mut self.prev_heat, self.heat);
        if self.heat == Heat::Pursuit && !matches!(prev_heat, Heat::Pursuit | Heat::Search | Heat::Busted) {
            self.last_known_v = pvel;
        }
        // (F-63286: +136 runs down; at 0 the brains' lost flag +773 is set again)
        if self.track_t > 0.0 {
            self.track_t -= dt;
            if self.track_t <= 0.0 {
                self.track_t = 0.0;
            }
        }
        let track_flag = self.track_t <= 0.0;
        let pfwd = p.state.forward();
        let pright = p.state.right();
        let speed = pvel.length();
        let mph = speed * MPH;
        let slip = p.state.wheels.iter().map(|w| w.slip).fold(0.0, f32::max);
        let damage = p.state.damage;
        let p_air = p.state.airborne_time;
        // (F-81388: the gyro's two-wheel state, +18, is not 0)
        let p_two = p.state.veh.as_ref().map_or(false, |v| v.two != 0);
        let tun = std::mem::take(&mut self.tun);
        let t = |k: &str, d: f32| tun.get(k, d);

        // ---- what the cruisers can see ---------------------------------------
        let detect = t("fDetectDist", 40.0);
        let vision = t("fVisionDist", 500.0);
        let mut seen = false;
        let mut nearest = f32::INFINITY;
        let mut n_cops = 0;
        // (F-94676 for a free cop - in no tactic, not the dispatch's -, F-52240: within fDetectDist in 3D and a clear
        // line from 2 m above the cop to 2 m above the target, F-69576; the half angle fDetectHalfAngle 180 is no cone.
        // The nearest such cop is the request cop of the tactic it starts or joins.)
        let dsp_cop = self.dsp.as_ref().map(|d| d.cop);
        let mut detector: Option<(f32, u32)> = None;
        for c in vehicles.cars.iter().filter(|c| c.cop && !c.goon && !pursuer(c) && Some(c.uid) != dsp_cop) {
            let d = c.state.pos.distance(ppos);
            if d <= detect && cop_sees(world, c, ppos, detect, 0.0) && detector.map_or(true, |b| d < b.0) {
                detector = Some((d, c.uid));
            }
        }
        // (the dispatch's cop is in its tactic: the ladder takes it as the request cop)
        if detector.is_none() {
            if let Some(c) = vehicles.cars.iter().find(|c| Some(c.uid) == dsp_cop) {
                let d = c.state.pos.distance(ppos);
                if d <= detect && cop_sees(world, c, ppos, detect, 0.0) {
                    detector = Some((d, c.uid));
                }
            }
        }
        let mut detected = detector.is_some();
        // (the tactic's seen test, F-16046: a target at fMovingSpeed or slower is seen; a moving one only when a probe
        // from half its height 7 m straight back hits something - F-74492 then grows +124 and zeroes +132, or the other
        // way round, +128 keeping last frame's +124)
        let probe_hit = {
            let o = ppos + Vec3::Y * (0.5 * (p.model.max.y - p.model.min.y));
            let back = -pfwd;
            world.collider.raycast(o, back, 7.0).is_some()
                || vehicles.cars.iter().filter(|c| !c.is_player).any(|c| {
                    let w = c.state.pos - o;
                    let along = w.dot(back);
                    let half = 0.5 * (c.model.max - c.model.min);
                    (-half.z..=7.0 + half.z).contains(&along) && (w - back * along).length() < half.x.max(1.0)
                })
        };
        let bike_down = p.model.bike && p.state.wipeout > 0.0;
        let seen_now = pvel.length() * MPH <= tun.get("fMovingSpeed", 1.0) || probe_hit;
        self.t_seen_prev = self.t_seen;
        if seen_now {
            self.t_seen += dt;
            self.t_unseen = 0.0;
        } else {
            self.t_unseen += dt;
            self.t_seen = 0.0;
        }
        for c in vehicles.cars.iter().filter(|c| pursuer(c)) {
            n_cops += 1;
            let d = c.state.pos.distance(ppos);
            nearest = nearest.min(d);
            if d < vision {
                let from = c.state.pos + Vec3::Y * 1.2;
                let to = ppos + Vec3::Y * 1.0;
                let dir = (to - from).normalize_or(Vec3::X);
                if d < 12.0 || world.collider.raycast(from, dir, (d - 2.0).max(0.1)).is_none() {
                    seen = true;
                }
            }
        }
        // (a free cop touching the player is a detecting cop as well: F-52240's fallback, the cop's contacts)
        let touched = (damage > self.last_damage + 0.004).then_some(()).and_then(|_| vehicles.cars.iter().find(|c| c.cop && !c.goon && !pursuer(c) && c.state.pos.distance(ppos) < 3.4)).map(|c| c.uid);
        if detector.is_none() {
            detector = touched.map(|u| (0.0, u));
        }
        detected |= touched.is_some();
        // (F-52240: with a tactic on the player that has a cop, offences count with no sight test; else only a
        // detecting cop counts them - and nothing is scored in the 5 s after a tactic on the player ended)
        let in_tactic = n_cops > 0 && !matches!(self.heat, Heat::None);
        let scoring = self.grace <= 0.0;
        self.grace = (self.grace - dt).max(0.0);
        let watching = scoring && (in_tactic || detected);

        // ---- offences ---------------------------------------------------------
        let mut breaking = false;
        let mut add = 0.0f32;
        // speeding: three bands above the limit
        // (F-36879: the road's limit in mph, `fSpeedLimitLow` where there is none; a band starts
        // at the larger of limit + offset and limit x (100 + percent) / 100)
        let limit = world.nav.as_ref().and_then(|n| n.speed_limit_at(ppos)).filter(|l| *l != 0.0).map_or(t("fSpeedLimitLow", 50.0), |l| l * MPH);
        let band = |off: &str, pct: &str, d_off: f32, d_pct: f32| (limit + t(off, d_off)).max(limit * (t(pct, d_pct) + 100.0) * 0.01);
        let lvl: u8 = if mph > band("fSpeedLimitHiOffset", "iSpeedLimitHiPrcnt", 45.0, 80.0) {
            3
        } else if mph > band("fSpeedLimitMidOffset", "iSpeedLimitMidPrcnt", 30.0, 50.0) {
            2
        } else if mph > band("fSpeedLimitLowOffset", "iSpeedLimitLowPrcnt", 15.0, 1.0) {
            1
        } else {
            0
        };
        if watching && lvl > 0 {
            breaking = true;
            self.offences |= LAW_SPEED;
            self.speed_top = self.speed_top.max(lvl);
            // (F-52240 / F-19359: a band higher than any paid pays its start points that tick and nothing else;
            // then the three bands share one seconds counter, the current band's rate. Dropping to no speeding
            // re-arms nothing: the started bits stay until the record is cleared)
            if lvl > self.speed_level {
                add += match lvl {
                    3 => t("iHighSpeedStartPoints", 600.0),
                    2 => t("iMidSpeedStartPoints", 250.0),
                    _ => t("iLowSpeedStartPoints", 130.0),
                };
                self.speed_level = lvl;
            } else {
                add += self.whole_seconds(0, dt) * match lvl {
                    3 => t("iHighSpeedPerSecPoints", 10.0),
                    2 => t("iMidSpeedPerSecPoints", 10.0),
                    _ => t("iLowSpeedPerSecPoints", 38.0),
                };
            }
        }
        // driving against the traffic
        self.nav_check -= dt;
        if self.nav_check <= 0.0 {
            self.nav_check = 0.2;
            self.against_traffic = false;
            if speed > 6.0 {
                if let Some(nav) = world.nav.as_ref() {
                    if let Some((li, _t, d)) = nav.nearest_link(ppos) {
                        if d < 5.0 && nav.links[li].dir().dot(pvel / speed) < -0.6 {
                            // a lane running our way right next to it means we are just near the centre line
                            let ok_lane = nav.links_near(ppos, 9.0).any(|j| {
                                let l = &nav.links[j];
                                l.dir().dot(pvel / speed) > 0.6 && l.point_at(l.project(ppos)).distance(ppos) < 4.0
                            });
                            self.against_traffic = !ok_lane;
                        }
                    }
                }
            }
        }
        if self.against_traffic {
            self.wrong_way += dt;
        } else {
            self.wrong_way = 0.0;
        }
        // (a started offence is on every frame its condition holds; the threshold only for its start)
        if watching && self.against_traffic && (self.wrong_active || self.wrong_way > t("fWrongWayThresholdTime", 1.2)) {
            breaking = true;
            self.offences |= LAW_WRONG_WAY;
            if !self.wrong_active {
                self.wrong_active = true;
                self.sec[1] = 0.0;
                add += t("iWrongWayStartPoints", 130.0);
            } else {
                add += self.whole_seconds(1, dt) * t("iWrongWayPerSecPoints", 25.0);
            }
        }
        // burn-outs
        if slip > 1.6 && speed < 9.0 {
            self.burnout += dt;
        } else {
            self.burnout = 0.0;
        }
        if watching && self.burnout > 0.0 && (self.burnout_active || self.burnout > t("fBurnOutThresholdTime", 1.5)) {
            breaking = true;
            self.offences |= LAW_BURNOUT;
            if !self.burnout_active {
                self.burnout_active = true;
                self.sec[2] = 0.0;
                add += t("iBurnOutStartPoints", 130.0);
            } else {
                add += self.whole_seconds(2, dt) * t("iBurnOutPerSecPoints", 25.0);
            }
        }
        // off the roads (F-90092: no polygon of the mesh with the ai_road material under the car)
        let off_road = crate::offroad::mesh().map_or(false, |m| !m.on_road(ppos));
        if watching && off_road {
            breaking = true;
            self.offences |= LAW_OFF_ROAD;
            if !self.off_active {
                // (its seconds counter is not reset at the start, as the speed bands')
                self.off_active = true;
                add += t("iOffRoadStartPoints", 50.0);
            } else {
                add += self.whole_seconds(4, dt) * t("iOffRoadPerSecPoints", 50.0);
            }
        }
        // driving on two wheels: start points, then per second
        if watching && p_two {
            breaking = true;
            self.offences |= LAW_TWO_WHEEL;
            if !self.two_active {
                self.two_active = true;
                self.sec[3] = 0.0;
                add += t("iTwoWheelStartPoints", 50.0);
            } else {
                add += self.whole_seconds(3, dt) * t("iTwoWheelPerSecPoints", 50.0);
            }
        }
        // a jump: in the air for longer than fAirJumpThresholdTime (F-29122, the car sim's air
        // time), paid once per jump
        if p_air > t("fAirJumpThresholdTime", 0.8) {
            if watching && !self.jump_paid {
                self.jump_paid = true;
                breaking = true;
                self.offences |= LAW_ROOF_JUMP;
                add += t("iJumpOffRoofTopPoints", 500.0);
            }
        } else {
            self.jump_paid = false;
        }
        // running a red light: approached on red, then drove into the box
        match vehicles.signals.ahead(ppos, pfwd, vehicles.clock) {
            Some((ji, stop, 0)) if stop < 25.0 => self.red_junction = Some(ji),
            Some((_, _, s)) if s != 0 => self.red_junction = None,
            _ => {}
        }
        if let (Some(ji), Some(inside)) = (self.red_junction, vehicles.signals.inside(ppos)) {
            if ji == inside {
                self.red_junction = None;
                if watching && speed > 4.0 {
                    breaking = true;
                    self.offences |= LAW_RED_LIGHT;
                    add += t("iRedLightPoints", 250.0);
                }
            }
        }
        // property damage
        if knocked > self.last_knocked {
            if watching {
                breaking = true;
                self.offences |= LAW_PROP;
                add += (knocked - self.last_knocked) as f32 * t("iDamagePropPoints", 250.0);
            }
            self.last_knocked = knocked;
        }
        // hitting other vehicles
        let mut hit_cop = false;
        if damage > self.last_damage + 0.004 {
            let hit = vehicles.cars.iter().filter(|c| !c.is_player && c.state.pos.distance(ppos) < 3.4).map(|c| c.cop).fold(None, |a: Option<bool>, cop| Some(a.unwrap_or(false) || cop));
            if let Some(cop) = hit {
                if cop {
                    // ramming a cruiser is always noticed
                    hit_cop = true;
                    breaking = true;
                    self.offences |= LAW_POLICE_VEHICLE;
                    add += t("iDamagePoliceVehPoints", 600.0);
                } else if watching {
                    breaking = true;
                    self.offences |= LAW_VEHICLE;
                    add += t("iDamageOtherVehPoints", 130.0);
                }
            }
        }
        self.last_damage = damage;

        // (AddPoints, F-19359: the score is held to iMaxLawBreakingScore, in a pull-over to iPursuitLawBreakScore - 1
        // until its bullhorn has run out, in a pursuit to iMax - 1)
        let max_score = t("iMaxLawBreakingScore", 1000.0);
        let pull = t("iPullOverLawBreakScore", 250.0);
        // (in a Pursuit or Double Pursuit 999 until it may escalate, vt+24: F-62213 / F-73000, F-38605 - not in its Delay, no radio line
        // playing, its may-escalate byte, not searching or arresting, no arrest timer running; 1000 in an Arrow Trap)
        let escalates = !self.radio_busy && self.may_escalate && !matches!(self.heat, Heat::Search | Heat::Busted) && self.arrest <= 0.0 && (self.kind != TacKind::Pursuit || self.flee_hold <= 0.0);
        let cap = match self.heat {
            // (F-19359 / F-98647: 599 until the bullhorn has sounded and run out and no arrest timer runs, F-38605)
            Heat::PullOver if self.bullhorn != Some(0.0) || self.arrest > 0.0 => t("iPursuitLawBreakScore", 600.0) - 1.0,
            _ if matches!(self.kind, TacKind::Pursuit | TacKind::PursuitDbl) && !escalates => max_score - 1.0,
            _ => max_score,
        };
        if breaking {
            self.score = (self.score + add).clamp(0.0, cap);
        }
        if scoring {
            // (F-52240 after the offences: under iPullOverLawBreakScore the score falls iNoLawBrokenPerSecPoints a
            // whole second while no offence is counted - iUndetectedPerSecPoints is read by nothing -; from it on it
            // climbs iPullOverPerSecPoints a whole second, offences or not, and never falls)
            if self.score > 0.0 && self.score < pull {
                if !breaking {
                    self.score = (self.score - self.whole_seconds(5, dt) * t("iNoLawBrokenPerSecPoints", 10.0)).max(0.0);
                    if self.score <= 0.0 {
                        self.offences = 0;
                    }
                }
            } else if self.score >= pull && self.score < max_score {
                let n = self.whole_seconds(6, dt);
                if n > 0.0 {
                    self.score = (self.score + n * t("iPullOverPerSecPoints", 10.0)).clamp(0.0, cap);
                }
            }
        }

        // ---- state machine ----------------------------------------------------
        let mut event = None;
        let arrest_speed = t("fArrestTgtThresholdSpd", 10.0) / MPH;
        match self.heat {
            Heat::None => {
                // (F-52240's ladder needs a detecting cop, the request cop: at iMaxLawBreakingScore an Arrow Trap, from
                // iPursuitLawBreakScore a Pursuit, from iPullOverLawBreakScore a Pull Over; StartTactic, F-33550, raises
                // the score to the tactic's floor - the zone's scripted chase too, its cop the nearest)
                let zone = std::mem::take(&mut self.zone_go);
                let first = detector.map(|d| d.1);
                let tt = t("fTrackTgtTime", 25.0);
                let kind = if detected && self.score >= max_score {
                    TacKind::ArrowTrap
                } else if zone || (detected && self.score >= t("iPursuitLawBreakScore", 600.0)) {
                    TacKind::Pursuit
                } else if detected && self.score >= pull {
                    TacKind::PullOver
                } else {
                    TacKind::None
                };
                if kind != TacKind::None {
                    if let Some(cops) = self.engage(gpu, world, vehicles, kind.cops(), if zone && !detected { None } else { first }, ppos, pfwd) {
                        self.start_tactic(kind, cops, true, tt);
                        if kind == TacKind::PullOver {
                            self.heat = Heat::PullOver;
                            self.bullhorn = None;
                            self.ticket_hold = None;
                            self.arrest = 0.0;
                            event = Some(Event::PullOver);
                        } else {
                            self.heat = Heat::Pursuit;
                            self.last_known = ppos;
                            self.fine = self.law_fines();
                            self.lost = 0.0;
                            event = Some(Event::Pursuit);
                        }
                    }
                }
            }
            Heat::PullOver => {
                // (F-22530's pull-over block: at iBullhornLawBreakScore the bullhorn and fBullhornTime; hitting a
                // police car turns it into a pursuit at once; else the score - held at 599 until the bullhorn has run
                // out - reaching iPursuitLawBreakScore does)
                if self.bullhorn.is_none() && self.score >= t("iBullhornLawBreakScore", 500.0) {
                    self.bullhorn = Some(t("fBullhornTime", 10.0));
                    log::info!("police: bullhorn");
                }
                if let Some(b) = self.bullhorn.as_mut() {
                    *b = (*b - dt).max(0.0);
                }
                if let Some(h) = self.ticket_hold.as_mut() {
                    // (StateTicket: the cop halted, fArrestStateTime - 0.5 s more when the car is damaged out -,
                    // then the tactic's result and COP_PULLOVER)
                    *h -= dt;
                    if *h <= 0.0 {
                        self.ticket_hold = None;
                        self.ticketing = true;
                        event = Some(Event::Ticket(self.law_fines() as i64));
                    }
                } else if hit_cop || self.score >= t("iPursuitLawBreakScore", 600.0) {
                    // (the pull-over's cop is the request cop; at 1000 an Arrow Trap)
                    let kind = if self.score >= max_score { TacKind::ArrowTrap } else { TacKind::Pursuit };
                    let first = self.cops.first().copied();
                    let cops = self.engage(gpu, world, vehicles, kind.cops(), first, ppos, pfwd).or_else(|| self.engage(gpu, world, vehicles, 1, first, ppos, pfwd));
                    if let Some(cops) = cops {
                        let kind = if cops.len() >= 3 { kind } else { TacKind::Pursuit };
                        self.start_tactic(kind, cops, true, t("fTrackTgtTime", 25.0));
                    }
                    self.heat = Heat::Pursuit;
                    self.last_known = ppos;
                    self.fine = self.law_fines();
                    self.lost = 0.0;
                    event = Some(Event::Pursuit);
                } else if self.arrest_step(nearest, t("fPullOverDist", 15.0), speed < arrest_speed, bike_down, t("fArrestTime", 1.0), t("fBikerWipeoutArrestDist", 50.0), dt) {
                    // (the pull-over's radius fPullOverDist, F-66453)
                    if self.ticket_hold.is_none() {
                        self.ticket_hold = Some(t("fArrestStateTime", 1.75) + if damage >= 0.999 { 0.5 } else { 0.0 });
                        // (what the ticket is for - the panel lists it: 1 reckless driving,
                        // 2 traffic violation, 4 destruction of property)
                        self.ticket_for = (if self.offences & (LAW_BURNOUT | LAW_VEHICLE | LAW_POLICE_VEHICLE) != 0 { 1 } else { 0 }) | (if self.offences & (LAW_SPEED | LAW_WRONG_WAY | LAW_RED_LIGHT) != 0 { 2 } else { 0 }) | (if self.offences & LAW_PROP != 0 { 4 } else { 0 });
                    }
                }
            }
            Heat::Pursuit => {
                self.fine = self.law_fines();
                // (the lead's brain, F-54688 outside the search: seen within fVisionDist with a clear line - unseen 0
                // and the last known place / velocity the target's; unseen with the lost flag +773: unseen time,
                // the last known kept once it passes fLostTgtThresholdTime; else the last known follows the target)
                let thr = t("fLostTgtThresholdTime", 1.5);
                // (each cop's brain; a Pursuit loses the target when its cop 0 does (F-68032), a squad when every cop
                // has (F-76774 / F-40655), and a squad's cops share the last known place and velocity)
                let mut lost_n = 0;
                let mut know = false;
                for u in self.cops.clone() {
                    let Some(c) = vehicles.cars.iter().find(|c| c.uid == u) else { continue };
                    let sees = cop_sees(world, c, ppos, vision, 0.0);
                    let e = self.eyes.entry(u).or_default();
                    if sees {
                        e.unseen = 0.0;
                    } else if track_flag {
                        e.unseen += dt;
                        if e.unseen > thr {
                            e.cone_t = 0.0;
                        }
                    }
                    let lost = track_flag && e.unseen > thr;
                    if lost {
                        lost_n += 1;
                    } else {
                        know = true;
                    }
                }
                let lead_lost = self.cops.first().and_then(|u| self.eyes.get(u)).is_some_and(|e| track_flag && e.unseen > thr);
                let all_lost = lost_n > 0 && lost_n == self.cops.len();
                if know && (self.kind.squad() || !lead_lost) {
                    self.last_known = ppos;
                    self.last_known_v = pvel;
                }
                self.lost = self.cops.first().and_then(|u| self.eyes.get(u)).map_or(0.0, |e| e.unseen);
                let into_search = if self.kind.squad() { all_lost } else { lead_lost };
                // (into Search; the pursuit's searcher is its cop 0 alone, no random turns, no turn angle (F-72765); the
                // Arrow Trap's: the cop nearest the target 0, the others +pi/2, -pi/2 in turn (F-17906); the Double
                // Pursuit's: the nearer 0, the other +-pi/2 at random (F-68396))
                if into_search {
                    self.heat = Heat::Search;
                    self.search = 0.0;
                    self.seen_time = 0.0;
                    self.radio = t("fMinPerRdoTime", 15.0);
                    self.srch.clear();
                    let mut order: Vec<(f32, u32)> = self.cops.iter().filter_map(|u| vehicles.cars.iter().find(|c| c.uid == *u)).map(|c| (c.state.pos.distance_squared(ppos), c.uid)).collect();
                    if self.kind.squad() {
                        order.sort_by(|a, b| a.0.total_cmp(&b.0));
                    } else {
                        order.truncate(1);
                    }
                    if let Some(nav) = world.nav.as_ref() {
                        let mut sign = 1.0;
                        for (k, (_, u)) in order.into_iter().enumerate() {
                            let angle = if k == 0 {
                                0.0
                            } else if self.kind == TacKind::PursuitDbl {
                                if crate::brain::rng().float() < 0.5 { std::f32::consts::FRAC_PI_2 } else { -std::f32::consts::FRAC_PI_2 }
                            } else {
                                let a = sign * std::f32::consts::FRAC_PI_2;
                                sign = -sign;
                                a
                            };
                            let mut rnd = || crate::brain::rng().float();
                            let pts = search_points(nav, world, self.last_known, self.last_known_v, t("fMinSetGoalTime", 1.0), true, false, angle, &mut rnd);
                            log::info!("police: cruiser {} search goals: {} (turn {:.2})", u, pts.len(), angle);
                            self.srch.push(CopSearch { uid: u, no_goals: pts.is_empty(), pts, angle, ..Default::default() });
                        }
                    }
                }
                // (the second-cop upgrade, F-22881: a free patrolling cop that detects the pursued target joins - a Pursuit
                // becomes a Double Pursuit, a Double Pursuit an Arrow Trap, the joining cop first - when the tactic may
                // escalate; at iMaxLawBreakingScore the ladder makes it an Arrow Trap with its own cop 0)
                if self.heat == Heat::Pursuit && self.may_escalate && !goons {
                    let up = if self.score >= max_score && self.kind != TacKind::ArrowTrap {
                        Some((TacKind::ArrowTrap, self.cops.first().copied()))
                    } else if let Some((_, u)) = detector.filter(|_| matches!(self.kind, TacKind::Pursuit | TacKind::PursuitDbl)) {
                        Some((if self.kind == TacKind::Pursuit { TacKind::PursuitDbl } else { TacKind::ArrowTrap }, Some(u)))
                    } else {
                        None
                    };
                    if let Some((kind, first)) = up {
                        if let Some(cops) = self.engage(gpu, world, vehicles, kind.cops(), first, ppos, pfwd) {
                            self.start_tactic(kind, cops, true, t("fTrackTgtTime", 25.0));
                        }
                    }
                }
                // (the pursuit's radius fSafeDist, F-54202)
                if self.arrest_step(nearest, t("fSafeDist", 30.0), speed < arrest_speed, bike_down, t("fArrestTime", 1.0), t("fBikerWipeoutArrestDist", 50.0), dt) {
                    {
                        self.heat = Heat::Busted;
                        self.arrest_posted = false;
                        // (F-56938: the arrest ends after fArrestStateTime, 0.5 s longer when the
                        // target is damaged out - the enter state F-69250 only stops a car whose damage
                        // points are under its MaxDamage and then sets the tactic byte +68)
                        self.busted_timer = t("fArrestStateTime", 1.75) + if damage >= 0.999 { 0.5 } else { 0.0 };
                        event = Some(Event::Busted(self.fine as i64));
                    }
                }
            }
            Heat::Search => {
                // (the second-cop upgrade runs for a searching tactic too: F-22881 tests only the tactic's +28 - a free
                // cop that sees the target makes the Pursuit a Double Pursuit, that an Arrow Trap, in CatchUp)
                if let (true, Some((_, u))) = (self.may_escalate && !goons && matches!(self.kind, TacKind::Pursuit | TacKind::PursuitDbl), detector) {
                    let kind = if self.kind == TacKind::Pursuit { TacKind::PursuitDbl } else { TacKind::ArrowTrap };
                    if let Some(cops) = self.engage(gpu, world, vehicles, kind.cops(), Some(u), ppos, pfwd) {
                        self.start_tactic(kind, cops, true, t("fTrackTgtTime", 25.0));
                        self.heat = Heat::Pursuit;
                        self.search = 0.0;
                        self.last_known = ppos;
                        self.last_known_v = pvel;
                    }
                }
                // (the searcher's brain, F-54688 in state 7: unseen time grows; with no search goals only +576 = 2; a
                // sighting within 15 m once started - fVisionDist before - finds it at once; else within
                // fVisionDist and fSearchHalfAngle of its forward for over fReacquireTgtThresholdTime. The last
                // known place and velocity stay.)
                let thr = t("fLostTgtThresholdTime", 1.5);
                let reacq = t("fReacquireTgtThresholdTime", 0.3);
                // (each searcher's brain; the tactic's Search, F-40917 / F-32025: found when some cop is not lost, or
                // is lost with no search goals and sees it within fVisionDist; the clock runs once a searcher started)
                let mut found = false;
                let mut started = false;
                for s in &self.srch {
                    let Some(c) = vehicles.cars.iter().find(|c| c.uid == s.uid) else { continue };
                    started |= s.started;
                    let e = self.eyes.entry(s.uid).or_default();
                    e.unseen += dt;
                    if !s.no_goals {
                        if cop_sees(world, c, ppos, if s.started { 15.0 } else { vision }, 0.0) {
                            e.cone_t = reacq;
                            e.unseen = 0.0;
                        } else if cop_sees(world, c, ppos, vision, t("fSearchHalfAngle", 45.0)) {
                            e.cone_t += dt;
                            if e.cone_t > reacq {
                                e.unseen = 0.0;
                            }
                        } else {
                            e.cone_t = 0.0;
                        }
                    }
                    let lost = track_flag && e.unseen > thr;
                    found |= !lost || (s.no_goals && cop_sees(world, c, ppos, vision, 0.0));
                }
                if self.srch.is_empty() {
                    found = !track_flag;
                }
                self.lost = self.srch.first().and_then(|s| self.eyes.get(&s.uid)).map_or(0.0, |e| e.unseen);
                if started && self.search == 0.0 && !self.goons {
                    // (the clock's first tick: the radio stopped, REPORT_SUSPECT_LOST)
                    self.radio_cue.push("REPORT_SUSPECT_LOST".into());
                }
                if found {
                    if self.search <= 0.0 {
                        self.heat = Heat::Pursuit;
                    } else if self.search < 2.0 {
                        // (F-79519 with 1 s: +773 off for a second; the clock goes on)
                        self.track_t = 1.0;
                        self.search += dt;
                    } else {
                        self.track_t = 20.0;
                        if !self.goons {
                            self.radio_cue.push(if self.search / t("fSearchMaxTime", 35.0) < 0.5 { "REPORT_SUSPECT_SPOTTED" } else { "REPORT_SUSPECT_REACQUIRE" }.into());
                        }
                        self.search = 0.0;
                        self.heat = Heat::Pursuit;
                    }
                    if self.heat == Heat::Pursuit {
                        self.eyes.clear();
                        self.srch.clear();
                        self.tacs.clear();
                        self.sq = Squad { hyst: 2.0, ..Default::default() };
                    }
                } else {
                    if started {
                        self.search += dt;
                    }
                    if self.search > t("fSearchMaxTime", 35.0) {
                        self.clear();
                        self.grace = 5.0;
                        event = Some(Event::Escaped);
                    }
                }
            }
            Heat::Busted => {
                self.busted_timer -= dt;
                if self.busted_timer <= 0.0 && !self.arrest_posted {
                    // (StateArrest ran its fArrestStateTime: the tactic succeeded, COP_ARREST -> CopArrest.sco)
                    self.arrest_posted = true;
                    event = Some(Event::ArrestScene);
                } else if self.busted_timer <= 0.0 {
                    // (step 19d: the payback's chasers stay until their script takes them away)
                    let goons = self.goons;
                    self.clear();
                    self.grace = 5.0;
                    self.goons = goons;
                    // (F-22530: the arrest tactic succeeded - for a cops' tactic, not the payback's
                    // goons, a race running gets raceOverCommand `returnToCruise`)
                    self.arrest_over = !goons;
                    // the cruisers drive off
                    vehicles.cars.retain(|c| !c.cop || c.state.pos.distance(ppos) > 60.0);
                }
            }
        }

        // (result 3, a tactic cop gone: the manager loses the police, F-22530)
        if gone && matches!(self.heat, Heat::Pursuit | Heat::Search | Heat::PullOver) {
            log::info!("police: a cruiser of the {:?} is gone - the police are lost", self.kind);
            self.clear();
            self.grace = 5.0;
            event = Some(Event::Escaped);
        }

        // (a ticket or a lost pull-over: the tactic is over)
        if self.heat == Heat::None && self.kind > TacKind::Dispatch {
            self.kind = TacKind::None;
            self.cops.clear();
        }

        // ---- Dispatch to Location (F-86831 starts it, F-87106 runs it) --------
        // (a DISPATCH trigger: nothing when the player is in a tactic; the cop the nearest not underground and in no
        // tactic, no distance limit; the location the player's place now; no score change)
        if let Some(call) = self.dispatch_call.take() {
            if self.heat == Heat::None && self.kind == TacKind::None && !goons {
                let cop = vehicles.cars.iter().filter(|c| c.cop && !c.goon).min_by(|a, b| a.state.pos.distance_squared(ppos).total_cmp(&b.state.pos.distance_squared(ppos))).map(|c| c.uid);
                if let Some(cop) = cop {
                    log::info!("police: dispatch {} - cruiser {}", call, cop);
                    self.kind = TacKind::Dispatch;
                    self.cops.clear();
                    self.dsp = Some(Dispatch { cop, st: 0, t: 0.0, loc: ppos, call, crumbs: Vec::new(), crumb_t: 1.0, seen_t: 0.0, trail: false, routed: false, arrived: false });
                }
            }
        }
        if self.kind != TacKind::Dispatch || self.heat != Heat::None {
            self.dsp = None;
        }
        if let Some(mut d) = self.dsp.take() {
            let cop = vehicles.cars.iter().find(|c| c.uid == d.cop);
            let mut result = 0;
            d.t += dt;
            // (the player's trail: a point a second, 15 at most, while the cop has not reached the place)
            if d.st < 3 && d.crumbs.len() < 15 && d.crumb_t > 0.0 {
                d.crumb_t -= dt;
                if d.crumb_t <= 0.0 {
                    d.crumb_t = 1.0;
                    d.crumbs.push(ppos);
                }
            }
            match cop {
                None => result = 3,
                Some(c) => {
                    // (Respond / WaitAtLocation: the cop sees the target within fRdoVicinityDist, line of sight, over 0.5 s)
                    if d.st >= 2 && cop_sees(world, c, ppos, t("fRdoVicinityDist", 100.0), 0.0) {
                        d.seen_t += dt;
                        if d.seen_t > 0.5 {
                            result = 1;
                        }
                    } else {
                        d.seen_t = 0.0;
                    }
                }
            }
            if result == 0 {
                match d.st {
                    // WaitToDispatch (F-64066): fDispatchRdoDelayTime and no radio line playing
                    0 if d.t > t("fDispatchRdoDelayTime", 3.0) && !self.radio_busy => {
                        d.st = 1;
                        d.t = 0.0;
                        // (Dispatch F-44635: the DISPATCH line of the trigger's sound record)
                        self.radio_cue.push(d.call.clone());
                    }
                    // (F-52633: on to Respond when the line has played)
                    1 if d.t > 0.0 && !self.radio_busy => {
                        d.st = 2;
                        d.t = 0.0;
                        d.routed = false;
                        d.arrived = false;
                        // (Respond F-81858: REPORT_ACCEPT_CALL, GoToPosition to the place, siren on)
                        self.radio_cue.push("REPORT_ACCEPT_CALL".into());
                    }
                    2 if d.arrived => {
                        // (WaitAtLocation F-52732: along the player's trail, else a search at once, +-pi/2 at random)
                        d.st = 3;
                        d.t = 0.0;
                        d.routed = false;
                        d.arrived = false;
                        d.trail = !d.crumbs.is_empty();
                        if !d.trail {
                            self.dispatch_search(world, vehicles, d.cop, t("fMinSetGoalTime", 1.0));
                        }
                    }
                    3 => {
                        // (F-92001: fDispatchLocWaitTime at the place ends it; at the trail's last crumb a search)
                        if d.t > t("fDispatchLocWaitTime", 30.0) {
                            result = 2;
                        } else if d.trail && d.arrived {
                            d.trail = false;
                            if let Some(&last) = d.crumbs.last() {
                                self.last_known = last;
                            }
                            self.dispatch_search(world, vehicles, d.cop, t("fMinSetGoalTime", 1.0));
                        }
                    }
                    _ => {}
                }
            }
            match result {
                // (F-22530: a dispatch's result 1 starts a Pursuit with its cop)
                1 => {
                    log::info!("police: dispatch cruiser {} sees the player - pursuit", d.cop);
                    self.srch.clear();
                    self.start_tactic(TacKind::Pursuit, vec![d.cop], true, t("fTrackTgtTime", 25.0));
                    self.heat = Heat::Pursuit;
                    self.last_known = ppos;
                    self.fine = self.law_fines();
                    self.lost = 0.0;
                    event = Some(Event::Pursuit);
                }
                // (2: 30 s at the place, 3: its cop gone - the record cleared, 5 s grace)
                2 | 3 => {
                    log::info!("police: dispatch over ({})", result);
                    self.clear();
                    self.grace = 5.0;
                }
                _ => self.dsp = Some(d),
            }
        }
        let dsp_cop = self.dsp.as_ref().map(|d| d.cop);

        // ---- the squad tactics: the Arrow Trap's and the Double Pursuit's FSM (F-76774 / F-40655) ---------------
        let members = self.cops.clone();
        let pursuer = |c: &crate::vehicles::CarInstance| members.contains(&c.uid);
        let safe = t("fSafeDist", 30.0);
        let along = t("fAlongSideDist", 5.0);
        let moving_v = t("fMovingSpeed", 1.0) * 0.447;
        let t_moving = speed > moving_v;
        // (F-42951: false only for a group cop - not a goon - while a race runs: then the brains plan the plain way and
        // the squad never swipes)
        let ed78 = !(self.in_race && !goons);
        let mut swipes: HashMap<u32, crate::ai::Overrides> = HashMap::new();
        if self.kind.squad() && self.heat == Heat::Pursuit && self.flee_hold <= 0.0 && self.cops.len() == self.kind.cops() {
            let arrow = self.kind == TacKind::ArrowTrap;
            // (closing(r): the cop moving, faster than the target and heading at it within 25.8 deg, within r)
            let closing = |c: &crate::vehicles::CarInstance, r: f32| {
                let (to, cv) = (ppos - c.state.pos, c.state.vel);
                let cs = cv.length();
                (r == 0.0 || to.length() < r) && cs > moving_v && cs > speed && to.normalize_or_zero().dot(cv / cs.max(1e-6)) > 0.9
            };
            let any_safe = vehicles.cars.iter().filter(|c| pursuer(c)).any(|c| c.state.pos.distance(ppos) < safe);
            // (the formation, F-80532 / F-21534: the right slot cop A's route point when it is on the target's right,
            // else 5 m to the right; the left cop B's when it is not, else 5 m to the left; the Arrow Trap's third 5 m
            // behind. The cops go to the slots of the smallest sum of squared distances, the flanks AlongSide (side 1 /
            // side 0), the third Pursue - sent once a change has been wanted over 1 s)
            let formation = |sq: &mut Squad, tacs: &mut HashMap<u32, CopTac>, vehicles: &Vehicles, dt: f32| -> bool {
                let route = |u: Option<u32>| {
                    let c = vehicles.cars.iter().find(|c| Some(c.uid) == u)?;
                    Some(c.ai.as_ref().and_then(|a| a.brain.0.as_ref()).and_then(|b| b.heading_for().1).unwrap_or(c.state.pos))
                };
                let right_of = |q: Vec3| pright.dot(q - ppos) > 0.0;
                let r = route(sq.slots[0]).filter(|&q| right_of(q)).unwrap_or(ppos + pright * along);
                let l = route(sq.slots[1]).filter(|&q| !right_of(q)).unwrap_or(ppos - pright * along);
                let slots = [r, l, ppos - pfwd * along];
                let pos: Vec<Vec3> = members.iter().map(|u| vehicles.cars.iter().find(|c| c.uid == *u).map_or(Vec3::ZERO, |c| c.state.pos)).collect();
                let perms: &[[usize; 3]] = if arrow { &[[0, 1, 2], [0, 2, 1], [1, 0, 2], [1, 2, 0], [2, 0, 1], [2, 1, 0]] } else { &[[0, 1, 2], [1, 0, 2]] };
                let mut best = (f32::INFINITY, perms[0]);
                for pm in perms {
                    let sum: f32 = (0..pos.len()).map(|k| pos[k].distance_squared(slots[pm[k]])).sum();
                    if sum < best.0 {
                        best = (sum, *pm);
                    }
                }
                let want = |k: usize| match best.1[k] {
                    0 => (Tac::AlongSide, true),
                    1 => (Tac::AlongSide, false),
                    _ => (Tac::CatchUp, false),
                };
                let changed: Vec<usize> = (0..pos.len()).filter(|&k| {
                    let (st, side) = want(k);
                    tacs.get(&members[k]).map_or(true, |tc| tc.st != st || (st == Tac::AlongSide && tc.side != side))
                }).collect();
                if changed.is_empty() {
                    sq.hyst = 0.0;
                    return false;
                }
                sq.hyst += dt;
                if sq.hyst <= 1.0 {
                    return false;
                }
                sq.hyst = 0.0;
                for k in changed {
                    let (st, side) = want(k);
                    let tc = tacs.entry(members[k]).or_default();
                    *tc = CopTac { st, side, ..Default::default() };
                }
                // (F-80532's tail: +172 the trailing cop, +164 the side-1 cop, +168 the other)
                sq.slots = [None; 3];
                for k in 0..pos.len() {
                    sq.slots[best.1[k]] = Some(members[k]);
                }
                true
            };
            let mut sq = std::mem::take(&mut self.sq);
            let mut tacs = std::mem::take(&mut self.tacs);
            if sq.halt {
                // Halt (F-58305 / F-16041): the cops within fSafeDist Stop again, x 0.5 when closing; none within it, or
                // the target over fArrestTgtThresholdSpd -> CatchUp
                if !any_safe || mph > t("fArrestTgtThresholdSpd", 10.0) {
                    sq.halt = false;
                    sq.begun = false;
                } else {
                    for c in vehicles.cars.iter_mut().filter(|c| pursuer(c) && c.state.pos.distance(ppos) < safe) {
                        let tc = tacs.entry(c.uid).or_default();
                        if tc.st != Tac::Halt {
                            *tc = CopTac { st: Tac::Halt, ..Default::default() };
                        }
                        if closing(c, 0.0) {
                            c.state.vel *= 0.5;
                        }
                    }
                }
            }
            if !sq.halt {
                if !sq.begun {
                    // (CatchUp's enter, F-81752 / F-86874: slots -1, hysteresis 2, the formation at once, lock 5 s)
                    sq = Squad { begun: true, hyst: 2.0, ..Default::default() };
                    formation(&mut sq, &mut tacs, vehicles, 0.0);
                    sq.lock = 5.0;
                } else {
                    // (F-47983 / F-41674: once the lock has run out the formation every tick, the lock again on a send)
                    sq.lock -= dt;
                    if sq.lock < 0.0 && formation(&mut sq, &mut tacs, vehicles, dt) {
                        sq.lock = 5.0;
                    }
                }
                // the flank swipes (only when F-42951 holds for cop A): the Arrow Trap's within 10 m, behind the
                // target, A with it on its left 135 - 160 deg off its right axis (after REPORT_MANEUVER_CLOSE and
                // 0.5 s), B with it on its right 20 - 45 deg; the Double Pursuit's within 15 m, 160 - 180 / 0 - 20 deg
                let (near, a_win, b_win) = if arrow { (10.0, (135.0, 160.0), (20.0, 45.0)) } else { (15.0, (160.0, 180.0), (0.0, 20.0)) };
                let geo = |u: Option<u32>| -> Option<(f32, bool, bool, bool, f32)> {
                    let c = vehicles.cars.iter().find(|c| Some(c.uid) == u)?;
                    let to = ppos - c.state.pos;
                    Some((to.length(), c.state.forward().dot(to) > 0.0, c.state.right().dot(to) > 0.0, pvel.dot(c.state.vel) > 0.0, c.state.right().angle_between(to).to_degrees()))
                };
                if ed78 {
                    let mut a_in = false;
                    if let Some((d, ahead, on_right, same_dir, bearing)) = geo(sq.slots[0]) {
                        if d < near && same_dir && !(arrow && ahead) && !on_right && bearing > a_win.0 && bearing < a_win.1 {
                            a_in = true;
                            if !sq.latch {
                                sq.latch = true;
                                sq.swipe_t = 0.5;
                                if !goons {
                                    self.radio_cue.push("REPORT_MANEUVER_CLOSE".into());
                                }
                            } else {
                                sq.swipe_t = (sq.swipe_t - dt).max(0.0);
                                if sq.swipe_t <= 0.0 {
                                    swipes.insert(sq.slots[0].unwrap_or_default(), crate::ai::Overrides { steer: Some(-1.0), brake: Some(1.0), ..Default::default() });
                                }
                            }
                        }
                    }
                    if !a_in {
                        sq.swipe_t = 0.0;
                        sq.latch = false;
                    }
                    if let Some((d, ahead, on_right, same_dir, bearing)) = geo(sq.slots[1]) {
                        if d < near && same_dir && !(arrow && ahead) && on_right && bearing > b_win.0 && bearing < b_win.1 {
                            swipes.insert(sq.slots[1].unwrap_or_default(), crate::ai::Overrides { steer: Some(1.0), brake: Some(1.0), ..Default::default() });
                        }
                    }
                    // (every cop head-on within fHeadOnBlockDist: handbrake, full lock to the target's side)
                    for u in &members {
                        if let (Some(tc), Some((d, _, on_right, _, _))) = (tacs.get(u), geo(Some(*u))) {
                            if tc.head_on && d < t("fHeadOnBlockDist", 60.0) {
                                swipes.insert(*u, crate::ai::Overrides { handbrake: Some(true), steer: Some(if on_right { 1.0 } else { -1.0 }), ..Default::default() });
                            }
                        }
                    }
                }
                // (the Arrow Trap: a cop within fSafeDist of a target that has stopped -> Halt: those within it Stop,
                // x 0.5 when closing)
                if arrow && any_safe && !t_moving {
                    log::info!("police: arrow trap halts");
                    sq.halt = true;
                    for c in vehicles.cars.iter_mut().filter(|c| pursuer(c) && c.state.pos.distance(ppos) < safe) {
                        tacs.insert(c.uid, CopTac { st: Tac::Halt, ..Default::default() });
                        if closing(c, 0.0) {
                            c.state.vel *= 0.5;
                        }
                    }
                }
            }
            self.sq = sq;
            self.tacs = tacs;
        }

        // ---- cruisers ---------------------------------------------------------
        let chasing = matches!(self.heat, Heat::Pursuit | Heat::Search | Heat::PullOver | Heat::Busted);
        self.flee_hold = (self.flee_hold - dt).max(0.0);
        if !matches!(self.heat, Heat::Pursuit | Heat::PullOver | Heat::Search) {
            self.tacs.clear();
        }
        // (in the Search the tactic's other cops keep the command they had: their states do not change)
        let searching = self.heat == Heat::Search;
        let tactic = matches!(self.heat, Heat::Pursuit | Heat::PullOver | Heat::Search) && self.flee_hold <= 0.0;
        let squad = self.kind.squad();
        let pull = self.heat == Heat::PullOver;
        let period = t("fMinSetGoalTime", 1.0);
        let dropped = self.last_tspeed - speed;
        self.last_tspeed = speed;
        // (every cop of the tactic within r of the target: F-90575, F-36876)
        let within = |r: f32| vehicles.cars.iter().filter(|c| pursuer(c)).all(|c| c.state.pos.distance(ppos) < r);
        let (all_block, all_halt) = (within(0.5 * safe), within(safe - 1.0));
        let on_road = |q: Vec3| crate::offroad::mesh().map_or(true, |m| m.on_road(q));
        for c in vehicles.cars.iter_mut().filter(|c| pursuer(c) || Some(c.uid) == dsp_cop) {
            let is_dsp = Some(c.uid) == dsp_cop && !pursuer(c);
            let Some(ai) = c.ai.as_mut() else { continue };
            ai.chase_exact = false;
            if self.flee_hold > 0.0 && self.heat == Heat::Pursuit && !is_dsp {
                // (the Delay state: brain command 1, halt, until its time is up)
                ai.chase = Some((c.state.pos, Vec3::ZERO));
                ai.ovr = Some(crate::ai::Overrides { brake: Some(1.0), throttle: Some(0.0), ..Default::default() });
            } else if (searching || is_dsp) && self.srch.iter().any(|s| s.uid == c.uid) {
                // ---- the search (brain state 7: F-57507, F-15801, the route F-55529) ----
                let Some(s) = self.srch.iter_mut().find(|s| s.uid == c.uid) else { continue };
                if ai.brain.0.is_none() {
                    if let Some(tu) = c.model.aitunes.as_ref() {
                        let tune = std::sync::Arc::new(mcla_formats::rage::aitune::AiTune::lerp(&tu.base, &tu.mods, c.perf));
                        ai.tune = Some(tune.clone());
                        ai.brain.0 = Some(Box::new(crate::brain::Brain::police(tune)));
                    }
                }
                ai.ovr = None;
                s.t += dt;
                let mesh = crate::offroad::mesh();
                let nav_ok = |p: Vec3| -> Option<Vec3> {
                    match mesh {
                        None => Some(p),
                        Some(m) if m.on_mesh(p) => Some(p),
                        Some(m) => m.teleport_target(p),
                    }
                };
                if !s.routed && !s.pts.is_empty() {
                    // (GoToPosition F-19329: off the mesh a probe 3 m up and down and F-45363; then it must be over
                    // the mesh. AppendGoal F-13831: a point not nav_ok is left out.)
                    let mut p0 = s.pts[0].0;
                    if !mesh.map_or(true, |m| m.on_mesh(p0)) && nav_ok(p0).is_none() {
                        if let Some(h) = world.collider.raycast(p0 + Vec3::Y * 3.0, -Vec3::Y, 6.0) {
                            p0 = nav_ok(h.point).unwrap_or(h.point);
                        }
                    } else if let Some(q) = nav_ok(p0) {
                        p0 = q;
                    }
                    if mesh.map_or(true, |m| m.on_mesh(p0)) {
                        let mut g = vec![p0];
                        g.extend(s.pts[1..].iter().filter_map(|q| nav_ok(q.0)));
                        if let Some(b) = ai.brain.0.as_mut() {
                            b.set_route(&g, -1.0, Vec3::ZERO);
                        }
                        s.routed = true;
                    }
                }
                let (idx, goal) = ai.brain.0.as_ref().map_or((0, None), |b| b.heading_for());
                // (+580: within 8 m of the goal it heads for, F-74140)
                s.near_t = if goal.is_some_and(|g| g.distance_squared(c.state.pos) < 64.0) { s.near_t + dt } else { 0.0 };
                let n = s.pts.len();
                if n > 1 {
                    if idx == 1 || s.near_t > 5.0 {
                        s.started = true;
                    } else if idx == n - 1 {
                        // (the rebuild from the last point; the direction is still the target's last known velocity)
                        if let Some(nav) = world.nav.as_ref() {
                            let last = s.pts[n - 1].0;
                            let mut rnd = || crate::brain::rng().float();
                            s.pts = search_points(nav, world, last, self.last_known_v, period, false, s.random, s.angle, &mut rnd);
                            s.routed = false;
                            if s.pts.is_empty() {
                                s.no_goals = true;
                            }
                        }
                    }
                } else if n == 1 && (s.near_t > 5.0 || (s.t > 0.0 && ai.brain.0.as_ref().is_some_and(|b| b.route_done()))) {
                    s.started = true;
                }
                if let Some(b) = ai.brain.0.as_mut() {
                    b.allow_nitro = c.state.pos.distance(ppos) > t("fAllowNitroDist", 125.0);
                }
                let goal = goal.unwrap_or(self.last_known);
                ai.chase = Some((goal, Vec3::ZERO));
                ai.chase_exact = true;
                ai.target_speed = 85.0;
            } else if is_dsp {
                // ---- the dispatch's cop: GoToPosition to the place (Respond), then along the player's trail ----
                let Some(d) = self.dsp.as_mut() else { continue };
                if d.st < 2 {
                    continue;
                }
                if ai.brain.0.is_none() {
                    if let Some(tu) = c.model.aitunes.as_ref() {
                        let tune = std::sync::Arc::new(mcla_formats::rage::aitune::AiTune::lerp(&tu.base, &tu.mods, c.perf));
                        ai.tune = Some(tune.clone());
                        ai.brain.0 = Some(Box::new(crate::brain::Brain::police(tune)));
                    }
                }
                ai.ovr = None;
                if !d.routed {
                    let g = if d.st == 3 { d.crumbs.clone() } else { vec![d.loc] };
                    if let Some(b) = ai.brain.0.as_mut() {
                        b.set_route(&g, -1.0, Vec3::ZERO);
                    }
                    d.routed = true;
                }
                let (_, goal) = ai.brain.0.as_ref().map_or((0, None), |b| b.heading_for());
                let end = if d.st == 3 { d.crumbs.last().copied().unwrap_or(d.loc) } else { d.loc };
                d.arrived = ai.brain.0.as_ref().map_or(c.state.pos.distance(end) < 8.0, |b| b.route_done());
                ai.chase = Some((goal.unwrap_or(end), Vec3::ZERO));
                ai.chase_exact = true;
                ai.target_speed = 85.0;
            } else if tactic {
                // ---- the tactic's state and its brain (docs/reference/hookmen/pursuit_tactics.md) ----
                let tac = self.tacs.entry(c.uid).or_default();
                let (cpos, cvel, cfwd, cright) = (c.state.pos, c.state.vel, c.state.forward(), c.state.right());
                let cspeed = cvel.length();
                let to = ppos - cpos;
                let d = to.length();
                // (the brain's predicates, F-74140's helpers: in front of the cop, on its right, going the same way,
                // facing the same way, the angle off the cop's right axis, and "closing in": the cop moving, faster
                // than the target and heading at it within 25.8 deg)
                let ahead = cfwd.dot(to) > 0.0;
                let on_right = cright.dot(to) > 0.0;
                let same_dir = pvel.dot(cvel) > 0.0;
                let same_facing = pfwd.dot(cfwd) > 0.0;
                let bearing = cright.angle_between(to).to_degrees();
                let closing = |r: f32| (r == 0.0 || d < r) && cspeed > moving_v && cspeed > speed && to.normalize_or_zero().dot(cvel / cspeed.max(1e-6)) > 0.9;
                // (head-on, +774: in front, coming the other way, seen, travel directions more than fHeadOnAngle apart)
                let tdir = if t_moving { pvel.normalize_or_zero() } else { pfwd };
                let cdir = if cspeed > moving_v { cvel / cspeed } else { cfwd };
                let was_head_on = tac.head_on;
                tac.head_on = ahead && !same_dir && !same_facing && seen && tdir.angle_between(cdir).to_degrees() > t("fHeadOnAngle", 145.0);
                // (F-14848: beside the target on the side over the road, the right first; else the side the cop is on)
                let pick_side = || {
                    let (r, l) = (on_road(ppos + pright * along), on_road(ppos - pright * along));
                    if r { true } else if l { false } else { !on_right }
                };
                let enter = |tac: &mut CopTac, st: Tac| {
                    tac.st = st;
                    tac.t = 0.0;
                    tac.far_t = 0.0;
                    tac.side_t = 0.0;
                    tac.grow_t = 0.0;
                    tac.replan = 0.0;
                };
                tac.t += dt;
                let st = tac.st;
                if searching || squad {
                    // (no tactic state changes during the Search; a squad's commands come from its FSM)
                } else if pull {
                    // pull-over (F-36876; CatchUp F-91234, AvoidCollision F-18622 / F-69591, Halt F-82776)
                    if st != Tac::Halt && all_halt && !t_moving && (cspeed < 4.47 || !closing(0.0)) {
                        enter(tac, Tac::Halt);
                    } else {
                        match st {
                            Tac::CatchUp if tac.t > 0.5 && closing(50.0) => {
                                enter(tac, Tac::Avoid);
                                tac.side = pick_side();
                                // (on entering: the cop's velocity x 0.7, F-19124)
                                c.state.vel *= 0.7;
                            }
                            Tac::Avoid => {
                                tac.side_t += dt;
                                if tac.side_t >= 1.0 {
                                    tac.side_t = 0.0;
                                    tac.side = pick_side();
                                }
                                if tac.t > 0.5 && d >= 50.0 {
                                    enter(tac, Tac::CatchUp);
                                }
                            }
                            Tac::Halt => {
                                tac.grow_t = if t_moving && d > tac.d_prev { tac.grow_t + dt } else { 0.0 };
                                if tac.grow_t > 0.5 || d >= safe {
                                    enter(tac, Tac::CatchUp);
                                }
                            }
                            _ => {}
                        }
                    }
                    // (every update: a stopped target and the cop closing in within 7 m - its velocity halved)
                    if !t_moving && closing(7.0) {
                        c.state.vel *= 0.5;
                    }
                } else {
                    // pursuit (F-68032; CatchUp F-88702, AlongSide F-85137, Block F-85039, Halt F-84018)
                    if !matches!(st, Tac::Halt | Tac::Block) && dropped > 4.0 && d < 5.0 {
                        // (the target just stopped hard beside it)
                        enter(tac, Tac::Halt);
                    } else {
                        match st {
                            Tac::CatchUp if d < safe => {
                                if !t_moving {
                                    enter(tac, Tac::Halt);
                                } else if tac.t > 3.0 && same_dir && ahead && bearing > 75.0 && bearing < 105.0 {
                                    enter(tac, Tac::AlongSide);
                                    tac.side = pick_side();
                                }
                            }
                            Tac::AlongSide => {
                                if d >= 1.5 * along {
                                    tac.far_t += dt;
                                    if tac.far_t > 10.0 {
                                        enter(tac, Tac::CatchUp);
                                    }
                                } else {
                                    tac.far_t = 0.0;
                                    tac.side_t += dt;
                                    if tac.side_t >= 1.0 {
                                        tac.side_t = 0.0;
                                        tac.side = pick_side();
                                    }
                                }
                                // (on its side - the target on the cop's other hand -, level or ahead, facing the same way)
                                if tac.st == Tac::AlongSide && same_facing && on_right != tac.side && !ahead {
                                    enter(tac, Tac::Block);
                                }
                            }
                            Tac::Block if !all_block => enter(tac, Tac::CatchUp),
                            Tac::Halt if !(d < safe && speed * MPH < t("fArrestTgtThresholdSpd", 10.0)) => enter(tac, Tac::CatchUp),
                            _ => {}
                        }
                    }
                }
                if tac.st != st {
                    log::info!("police: cruiser {} {:?} -> {:?} ({:.0} m)", c.uid, st, tac.st, d);
                }
                tac.d_prev = d;
                // ---- the goal plan (F-67303; docs/reference/hookmen/goal_speed.md). The cop is driven by the racers'
                // brain (brain.rs, mcBrainPolice is an mcBrainRacing): its goals, a 5 m radius each, and the speed to
                // reach the last with; the speed on the way is the race line's speed profile. Only the PullOver,
                // Pursuit and AlongSide brains plan (slot 22); the stopping states keep the route they had and their
                // overrides decide. ----
                if ai.brain.0.is_none() {
                    if let Some(tu) = c.model.aitunes.as_ref() {
                        let tune = std::sync::Arc::new(mcla_formats::rage::aitune::AiTune::lerp(&tu.base, &tu.mods, c.perf));
                        ai.tune = Some(tune.clone());
                        ai.brain.0 = Some(Box::new(crate::brain::Brain::police(tune)));
                    }
                }
                // (the port's other cops of a search keep the route they had - the original's pursuit has one cop)
                let plans = matches!(tac.st, Tac::CatchUp | Tac::AlongSide | Tac::Avoid) && !searching;
                let per = if tac.head_on { 0.5 * period } else { period };
                tac.replan = (tac.replan - dt).max(0.0);
                let (empty, done) = ai.brain.0.as_ref().map_or((true, false), |b| (b.goal_count() == 0, b.route_done()));
                let turned = tac.plan_fwd != Vec3::ZERO && tac.plan_fwd.angle_between(pfwd) > 0.5236;
                let changed = (speed - tac.plan_speed).abs() > 6.705;
                let ended_head_on = was_head_on && !tac.head_on;
                let quick = tac.replan < 0.3 * per && (turned || changed);
                if plans && (tac.replan <= 0.0 || done || ended_head_on || quick || empty) {
                    let ho = if tac.head_on { 2.0 } else { 1.0 };
                    let nav = world.nav.as_ref();
                    // (F-72689 over the mesh, or F-45363 a polygon within 10 m)
                    let nav_ok = |q: Vec3| crate::offroad::mesh().map_or(true, |m| m.on_mesh(q) || m.teleport_target(q).is_some());
                    // F-19210: the lane within 5 m of q gives the direction, scaled to the target's speed - its velocity
                    // itself when there is none, it is a junction's, or it is 45 deg or more off the velocity
                    let dir_at = |q: Vec3| -> Vec3 {
                        let lane = nav.and_then(|n| crate::vehicles::traffic::snap_to_lane(n, q, 5.0).0.filter(|h| n.lane_junction.get(h.lane).map_or(true, |j| j.is_none())));
                        match lane.map(|h| h.dir.normalize_or_zero()).filter(|l| *l != Vec3::ZERO) {
                            Some(l) => {
                                let l = if l.dot(pvel) < 0.0 { -l } else { l };
                                if l.angle_between(pvel) < 0.785_398 { l * speed } else { pvel }
                            }
                            None => pvel,
                        }
                    };
                    // the two-goal chain from b, k seconds of D each; a goal off the mesh is moved back and forth
                    // along D by a quarter of k (4 tries each way, from the goal; D read again at the last point tried)
                    let chain = |b: Vec3, k: f32| -> [Vec3; 2] {
                        let mut out = [b; 2];
                        let mut q = b;
                        for g in out.iter_mut() {
                            let p0 = q + dir_at(q) * k;
                            *g = p0;
                            q = p0;
                            if !nav_ok(p0) {
                                for j in 1..=4 {
                                    let dd = dir_at(q);
                                    q = p0 + dd * (k - 0.25 * j as f32);
                                    if nav_ok(q) {
                                        *g = q;
                                        break;
                                    }
                                    q = p0 + dd * (k + 0.25 * j as f32);
                                    if nav_ok(q) {
                                        *g = q;
                                        break;
                                    }
                                }
                            }
                        }
                        out
                    };
                    let (g, v): ([Vec3; 2], f32) = if quick && turned {
                        // the junction prediction: the road out of the junction the target is in that lies most along
                        // its heading (x / z from the junction's centre), at the middle of that road's lane
                        let pick = nav.and_then(|n| {
                            let hit = crate::vehicles::traffic::snap_to_lane(n, ppos, 5.0).0?;
                            let j = (*n.lane_junction.get(hit.lane)?)?;
                            let jc = n.junction_c.get(j)?;
                            let mut best: Option<(f32, usize)> = None;
                            for r in n.junction_roads(j) {
                                let z = &n.zones[r];
                                let d2 = glam::Vec2::new(z.rect_c[0] - jc[0], z.rect_c[1] - jc[1]).normalize_or_zero();
                                if d2 == glam::Vec2::ZERO {
                                    continue;
                                }
                                let sc = d2.dot(glam::Vec2::new(pfwd.x, pfwd.z));
                                if sc > 0.0 && best.map_or(true, |b| sc > b.0) {
                                    best = Some((sc, r));
                                }
                            }
                            let lane = n.zones[best?.1].lane?;
                            Some(n.curves.get(lane)?.eval(0.5).0)
                        });
                        let q = pick.filter(|&q| on_road(q) || nav_ok(q)).unwrap_or(ppos);
                        ([q, q], speed)
                    } else if quick {
                        ([ppos, ppos], speed)
                    } else {
                        // the target's own speed when it is moving, else 0 (F-85634)
                        let default_v = if t_moving { speed } else { 0.0 };
                        match (tac.st, pull) {
                            // Pursuit brain (F-15067): from the target, P and 2P (head-on 2P, 4P) along D; a stopped target:
                            // from fSafeDist off it towards the cop. Final speed (F-49680): 0 for a stopped target, full
                            // pace (-1) beyond 2 x fSafeDist, 1.2 x its speed when it is near, going the same way and
                            // ahead of the cop, else full pace
                            // (F-42951 false - a police cop in a race: from the target less P of its velocity, 1 s steps,
                            // the default final speed)
                            (Tac::CatchUp, false) if !ed78 => {
                                let b = if t_moving { ppos - pvel * per } else { ppos + (cpos - ppos).normalize_or(Vec3::X) * safe };
                                (chain(b, 1.0), default_v)
                            }
                            (Tac::CatchUp, false) => {
                                let b = if t_moving { ppos } else { ppos + (cpos - ppos).normalize_or(Vec3::X) * safe };
                                let v = if !t_moving { 0.0 } else if d >= 2.0 * safe { -1.0 } else if same_dir && ahead { 1.2 * speed } else { -1.0 };
                                (chain(b, per * ho), v)
                            }
                            // PullOver brain (F-51787): one point, both goals - the target; 15 m behind it stopped
                            // (fPullOverDist); driving the wrong way, P of its velocity on and 8 m to the side of its lane;
                            // off the mesh, back by P of its velocity up to 5 times
                            (Tac::CatchUp, true) => {
                                let lane_dir = nav.and_then(|n| crate::vehicles::traffic::snap_to_lane(n, ppos, 5.0).0).map(|h| h.dir.normalize_or_zero());
                                let mut q = match lane_dir {
                                    Some(tn) if self.against_traffic => ppos + pvel * per + Vec3::new(tn.z, 0.0, -tn.x) * 8.0,
                                    _ if !t_moving => ppos - pfwd * t("fPullOverDist", 15.0),
                                    _ => ppos,
                                };
                                if !nav_ok(q) {
                                    if let Some(k) = (1..=5).find(|&i| nav_ok(q - pvel * per * i as f32)) {
                                        q -= pvel * per * k as f32;
                                    }
                                }
                                ([q, q], default_v)
                            }
                            // AlongSide brain (F-13443): from fAlongSideDist beside the target, 1.15 P (head-on 2.3 P) along D
                            _ if !ed78 => {
                                let b = ppos + pright * if tac.side { along } else { -along } - pvel * per;
                                (chain(b, 1.0), default_v)
                            }
                            _ => {
                                let b = ppos + pright * if tac.side { along } else { -along };
                                (chain(b, 1.15 * per * ho), default_v)
                            }
                        }
                    };
                    tac.replan = per;
                    tac.plan_fwd = pfwd;
                    tac.plan_speed = speed;
                    tac.goals = g;
                    tac.speed = v;
                    if let Some(b) = ai.brain.0.as_mut() {
                        b.set_goals(&g, v);
                    }
                }
                // (brain +236: the race nitro logic only beyond fAllowNitroDist)
                if let Some(b) = ai.brain.0.as_mut() {
                    b.allow_nitro = d > t("fAllowNitroDist", 125.0);
                }
                // (the port's own driver when the car has no racing tune: at the goals in turn)
                let goal = if cpos.distance(tac.goals[0]) < 5.0 { tac.goals[1] } else { tac.goals[0] };
                ai.chase = Some((goal, Vec3::ZERO));
                ai.chase_exact = true;
                ai.target_speed = 85.0;
                // ---- the brain's overrides (F-88118 puts them over the driver's input) ----
                let away = Some(if on_right { -1.0 } else { 1.0 });
                ai.ovr = match (tac.st, pull) {
                    (Tac::CatchUp, false) if (!t_moving && d < safe) || (ahead && same_dir && d < 20.0 && cspeed > speed) => Some(crate::ai::Overrides { brake: Some(1.0), ..Default::default() }),
                    (Tac::AlongSide | Tac::Avoid, _) if (!t_moving && d < along + 1.0) || (t_moving && same_dir && !ahead && cspeed > speed && (!ed78 || d > 1.5 * along)) => Some(crate::ai::Overrides { brake: Some(1.0), ..Default::default() }),
                    // Block (F-56982): handbrake, no throttle, full lock towards the target
                    (Tac::Block, _) => Some(crate::ai::Overrides { handbrake: Some(true), throttle: Some(0.0), steer: Some(if on_right { 1.0 } else { -1.0 }), brake: None }),
                    // Halt (F-29519): stopped beside the target; steering away while closing in
                    (Tac::Halt, false) => Some(crate::ai::Overrides { brake: (d < safe).then_some(1.0), steer: if closing(safe) { away } else { None }, ..Default::default() }),
                    (Tac::Halt, true) => Some(crate::ai::Overrides { brake: Some(1.0), ..Default::default() }),
                    _ => None,
                };
                // (the squad's swipes and head-on blocks go over the brain's input)
                if let Some(o) = swipes.get(&c.uid) {
                    let mut v = ai.ovr.unwrap_or_default();
                    v.steer = o.steer.or(v.steer);
                    v.brake = o.brake.or(v.brake);
                    v.handbrake = o.handbrake.or(v.handbrake);
                    v.throttle = o.throttle.or(v.throttle);
                    ai.ovr = Some(v);
                }
            } else if let Some(ai) = Some(ai) {
                // (out of the tactics: the port's own driver again)
                ai.brain.0 = None;
                let target = match self.heat {
                    Heat::Search => (self.last_known, Vec3::ZERO),
                    _ => (ppos, pvel),
                };
                ai.ovr = None;
                if chasing && self.heat != Heat::Busted {
                    ai.chase = Some(target);
                    ai.target_speed = 22.0;
                } else if self.heat == Heat::Busted {
                    ai.chase = Some((c.state.pos, Vec3::ZERO));
                } else if ai.chase.is_some() {
                    ai.chase = None;
                    ai.path = None;
                    ai.link = None;
                    ai.target_speed = 14.0;
                }
            }
        }
        // (a cop in no tactic patrols: StartTactic ends the old one with flag 0 - its commands cleared)
        for c in vehicles.cars.iter_mut().filter(|c| c.cop && !c.goon && !pursuer(c) && Some(c.uid) != dsp_cop) {
            let Some(ai) = c.ai.as_mut() else { continue };
            if ai.brain.0.is_some() || ai.chase.is_some() || ai.ovr.is_some() {
                ai.brain.0 = None;
                ai.ovr = None;
                ai.chase = None;
                ai.chase_exact = false;
                ai.path = None;
                ai.link = None;
                ai.target_speed = 14.0;
                self.tacs.remove(&c.uid);
            }
        }
        // patrol: a cruiser put on a patrol route when no free one is about (the port's; the original's patrol cops
        // are the manager's units)
        let free = vehicles.cars.iter().any(|c| c.cop && !c.goon && !pursuer(c));
        self.spawn_timer -= dt;
        if self.spawn_timer <= 0.0 && !self.goons {
            match self.heat {
                Heat::None | Heat::PullOver | Heat::Pursuit | Heat::Search => {
                    if !free {
                        // a patrol route passing nearby, else any street
                        let near = self.paths.iter().flatten().copied().filter(|q| {
                            let d = q.distance(ppos);
                            d > 150.0 && d < 380.0
                        }).min_by(|a, b| a.distance(ppos).partial_cmp(&b.distance(ppos)).unwrap_or(std::cmp::Ordering::Equal));
                        let ok = match near {
                            Some(q) => self.spawn_cop(gpu, world, vehicles, ppos, pfwd, 120.0, 400.0, Some(q)),
                            None => self.rand() < 0.5 && self.spawn_cop(gpu, world, vehicles, ppos, pfwd, 150.0, 300.0, None),
                        };
                        self.spawn_timer = if ok { 20.0 } else { 6.0 };
                    } else {
                        self.spawn_timer = 10.0;
                    }
                }
                Heat::Busted => {}
            }
        }
        self.tun = tun;
        self.update_glows(gpu, vehicles);
        event
    }

    /// The tactic's arrest timer +108 (F-29906) and its test (F-36806): set past fArrestTime at once for a wiped-out
    /// biker with a cop of the tactic within fBikerWipeoutArrestDist; else, with a cop of the tactic within the
    /// tactic's radius + 1 m (F-43276), it grows while the target is under fArrestTgtThresholdSpd and seen this frame
    /// (+128 < +124) and holds otherwise; out of range it is zeroed only once the target has been unseen over 0.5 s.
    /// True when it is past fArrestTime. (+140 / +144 over 5 s also arrest in the original: not ported.)
    #[allow(clippy::too_many_arguments)]
    fn arrest_step(&mut self, nearest: f32, radius: f32, slow: bool, bike_down: bool, at: f32, bike_r: f32, dt: f32) -> bool {
        if bike_down && nearest < bike_r {
            self.arrest = at + 1.0;
        } else if nearest < radius + 1.0 {
            if self.t_seen_prev < self.t_seen && slow {
                self.arrest += dt;
            }
        } else if self.t_unseen > 0.5 {
            self.arrest = 0.0;
        }
        self.arrest > at
    }

    /// The fine (F-83946, the routine that fills the arrest form): the highest speed band's
    /// `iLawFines` entry, plus the entry of every other offence (bit n of the original's word
    /// = entry n; a hit on a cruiser sets 0x20 and 0x40, the second one is taken back off),
    /// plus for a pursuit `iPerSecPursuitFine` x its length (rounded half away from zero) and
    /// `iPullOverFleeFine` after running from a pull-over; cut to 1/4, 1/3, 1/2 below 25 %,
    /// 50 %, 75 % of the career, at least 100, at most `iMaxFine`.
    /// The whole seconds that counter `i` has run through (the original subtracts 1 s while the
    /// counter stands above 1 s).
    fn whole_seconds(&mut self, i: usize, dt: f32) -> f32 {
        self.sec[i] += dt;
        let mut n = 0.0;
        while self.sec[i] > 1.0 {
            self.sec[i] -= 1.0;
            n += 1.0;
        }
        n
    }

    fn law_fines(&self) -> f32 {
        let t = |i: usize| self.tun.array_at("iLawFines", i);
        let mut f = 0i32;
        if self.speed_top > 0 {
            f += t(self.speed_top as usize - 1);
        }
        for (flag, entry) in [(LAW_PROP, 4), (LAW_VEHICLE, 7), (LAW_POLICE_VEHICLE, 5) /* (0x20 + 0x40 - entry 6) */, (LAW_RED_LIGHT, 9), (LAW_BURNOUT, 11), (LAW_WRONG_WAY, 13), (LAW_ROOF_JUMP, 12), (LAW_TWO_WHEEL, 10), (LAW_OFF_ROAD, 3)] {
            if self.offences & flag != 0 {
                f += t(entry);
            }
        }
        if matches!(self.heat, Heat::Pursuit | Heat::Search | Heat::Busted) {
            let x = self.tun.get("iPerSecPursuitFine", 10.0) * self.chase_time;
            f += (if x < 0.0 { x - 0.5 } else { x + 0.5 }) as i32;
            if self.fled {
                f += self.tun.get("iPullOverFleeFine", 300.0) as i32;
            }
        }
        f = if self.progress < 0.25 {
            f / 4
        } else if self.progress < 0.5 {
            f / 3
        } else if self.progress < 0.75 {
            f / 2
        } else {
            f
        };
        if f < 100 {
            return 100.0;
        }
        f.min(self.tun.get("iMaxFine", 5000.0) as i32) as f32
    }

    fn clear(&mut self) {
        self.heat = Heat::None;
        self.score = 0.0;
        self.fine = 0.0;
        self.offences = 0;
        self.speed_level = 0;
        self.speed_top = 0;
        self.fled = false;
        // (F-88677 clears the whole record: the started bits and the counters too)
        self.off_active = false;
        self.two_active = false;
        self.wrong_active = false;
        self.burnout_active = false;
        self.sec = [0.0; 7];
        self.bullhorn = None;
        self.ticket_hold = None;
        self.ticketing = false;
        self.flee_hold = 0.0;
        self.arrest = 0.0;
        self.lost = 0.0;
        self.search = 0.0;
        self.srch.clear();
        self.kind = TacKind::None;
        self.cops.clear();
        self.eyes.clear();
        self.sq = Squad::default();
        self.dsp = None;
        self.spawn_timer = 12.0;
    }

    /// Roof light positions and colours (red / blue, alternating) of the cruisers with
    /// their lights on.
    pub fn light_bar(&self, vehicles: &Vehicles) -> Vec<(Vec3, [f32; 3])> {
        let mut out = Vec::new();
        if !self.lights_on() {
            return out;
        }
        let phase = (self.time * 5.0) as i32 % 2 == 0;
        for c in vehicles.cars.iter().filter(|c| c.cop) {
            let up = c.state.up();
            let right = c.state.right();
            let top = c.state.pos + up * 1.30 - c.state.forward() * 0.45;
            let (l, r) = if phase { ([1.0, 0.05, 0.05], [0.0, 0.0, 0.0]) } else { ([0.0, 0.0, 0.0], [0.1, 0.25, 1.0]) };
            if phase {
                out.push((top - right * 0.38, l));
            } else {
                out.push((top + right * 0.38, [0.1, 0.25, 1.0]));
            }
            let _ = r;
        }
        out
    }

    fn update_glows(&mut self, gpu: &Gpu, vehicles: &Vehicles) {
        let bar = self.light_bar(vehicles);
        let mut insts = [Inst { pos_scale: [0.0; 4], rot_tint: [0.0; 4], fill: [0; 4], ..Inst::NONE }; MAX_GLOWS];
        let n = bar.len().min(MAX_GLOWS);
        for (i, (p, c)) in bar.iter().take(n).enumerate() {
            insts[i] = Inst { pos_scale: [p.x, p.y, p.z, -0.55], rot_tint: [c[0], c[1], c[2], 2.2], fill: [0; 4], ..Inst::NONE };
        }
        if self.glow_buf.is_none() {
            self.glow_buf = Some(gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("police glows"),
                size: (std::mem::size_of::<Inst>() * MAX_GLOWS) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
        }
        if let Some(b) = self.glow_buf.as_ref() {
            gpu.queue.write_buffer(b, 0, bytemuck::cast_slice(&insts));
        }
        self.glow_count = n as u32;
    }

    /// Additive light-bar flares. Sets the glow pipeline.
    pub fn draw_glows<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>, gpu: &'a Gpu) {
        let Some(buf) = self.glow_buf.as_ref() else { return };
        if self.glow_count == 0 {
            return;
        }
        pass.set_pipeline(&gpu.pipeline_glow);
        pass.set_vertex_buffer(0, buf.slice(..));
        pass.draw(0..4, 0..self.glow_count);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cop(arrays: &[(&str, Vec<i32>)]) -> Police {
        let mut p = Police::new();
        for (k, v) in arrays {
            p.tun.arrays.insert(k.to_string(), v.clone());
        }
        p.tun.v.insert("iMaxFine".into(), 5000.0);
        p.tun.v.insert("iPerSecPursuitFine".into(), 10.0);
        p.tun.v.insert("iPullOverFleeFine".into(), 300.0);
        p
    }

    /// F-83946: the sum over the offences, the pursuit's seconds, the flee fine, the career cut
    /// and the clamp, as the routine reads them.
    #[test]
    fn fine_of_the_original() {
        let mut p = cop(&[("iLawFines", vec![100, 110, 120, 130, 140, 150, 160, 170, 180, 190, 200, 210, 220, 230])]);
        // one speed band (the highest, entry 2), nothing else: 120
        p.offences = LAW_SPEED;
        p.speed_top = 3;
        assert_eq!(p.law_fines(), 120.0);
        // plus a cruiser hit (entry 5, the 0x40 entry cancels) and a prop (entry 4)
        p.offences |= LAW_POLICE_VEHICLE | LAW_PROP;
        assert_eq!(p.law_fines(), 120.0 + 150.0 + 140.0);
        // a 20.04 s pursuit adds round(200.4) = 200, running from the pull-over 300
        p.heat = Heat::Pursuit;
        p.chase_time = 20.04;
        p.fled = true;
        assert_eq!(p.law_fines(), 410.0 + 200.0 + 300.0);
        // the career cuts it
        p.progress = 0.6;
        assert_eq!(p.law_fines(), 455.0);
        p.progress = 0.3;
        assert_eq!(p.law_fines(), 303.0);
        p.progress = 0.1;
        assert_eq!(p.law_fines(), 227.0);
        // never under 100, never over iMaxFine
        let mut q = cop(&[("iLawFines", vec![100; 14])]);
        q.progress = 0.0;
        q.offences = LAW_SPEED;
        q.speed_top = 1;
        assert_eq!(q.law_fines(), 100.0);
        q.progress = 1.0;
        q.heat = Heat::Pursuit;
        q.chase_time = 1000.0;
        assert_eq!(q.law_fines(), 5000.0);
    }
}

/// The arrest's scene (CopArrest.sco 0xc33 - 0xeef): one cop in the latest tactic -> `cut_cop_arrest` or `_07`
/// (RAND_INT_RANGE(0, 1), upper bound included: F-32828), more -> `_02`, `_03` or `_08` (RAND_INT_RANGE(0, 2)).
/// `r` stands in for the game's random number. (The payback's goons scenes and the wiped-out biker's
/// `cut_cop_arrest_bikestand` are not ported.)
pub fn arrest_scene(cops: usize, r: u32) -> &'static str {
    if cops <= 1 {
        ["cut_cop_arrest_07", "cut_cop_arrest"][(r % 2) as usize]
    } else {
        ["cut_cop_arrest_02", "cut_cop_arrest_03", "cut_cop_arrest_08"][(r % 3) as usize]
    }
}

/// The pull-over scenes' events on the player's racer (slot 0 / `mover_drv00_car`), from their generated scene
/// scripts: (time, what). The walk's `gesture_left` (event 1, F-56242: racer+684 = 1, the left round robin), the
/// ticket scenes' `gesture_copfidget` (event 52, F-41149: 17, `drv_gest_disapnt`) and the left window (events 29 / 31);
/// every gesture has duration 0 (no end call: the clip plays out).
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum SceneAct {
    Gesture(crate::drivers::Gesture),
    LeftWindow(bool),
}

pub fn pullover_acts(scene: &str) -> &'static [(f32, SceneAct)] {
    use crate::drivers::Gesture;
    const WALK: [(f32, SceneAct); 1] = [(3.6333, SceneAct::Gesture(Gesture::Left))];
    const FIDGET: [(f32, SceneAct); 3] = [(0.0, SceneAct::Gesture(Gesture::CopFidget)), (0.0, SceneAct::LeftWindow(true)), (7.3, SceneAct::LeftWindow(false))];
    const BIKE: [(f32, SceneAct); 1] = [(0.0, SceneAct::Gesture(Gesture::CopFidget))];
    let s = scene.to_ascii_lowercase();
    if s.ends_with("cut_cop_walk_02") {
        &WALK
    } else if s.ends_with("cut_cop_fidget_02") || s.ends_with("sc_cut_cop_ticket_suv") {
        &FIDGET
    } else if s.ends_with("cut_cop_fidget_bike") {
        &BIKE
    } else {
        &[]
    }
}

/// When the arrest scene's `gesture_cophandsup` puts the player's hands up: (start, duration) in the scene's
/// seconds, from the generated scene scripts' event records (slot 0 = the player). `cut_cop_arrest` aims its
/// one at slot 1, the first cop car, whose driver is off - so no hands up there.
pub fn arrest_hands_up(scene: &str) -> Option<(f32, f32)> {
    Some(match scene.rsplit('/').next().unwrap_or(scene).to_ascii_lowercase().as_str() {
        "cut_cop_arrest_02" => (1.8333, 4.1667),
        "cut_cop_arrest_03" => (1.3333, 3.6667),
        "cut_cop_arrest_07" => (3.5, 1.5),
        "cut_cop_arrest_08" => (0.0, 6.6667),
        _ => return None,
    })
}

/// The police scenes' place (CopPullover.sco 0x54c, CopArrest.sco 0xa1f): the player's car put upright where it
/// stands with its heading (Racer_InitializeResetPosition, F-40200, and ResetWithDamageIntact, F-67152), and the
/// scene's frame - that heading alone, snapped to the ground from 5 m above to 5 m below and leaned with it when it
/// is off by more than 0.002 rad (SetGameCoordSysBindingCopyRacer, F-94483: F-88556, F-33119); never the car's
/// own pitch or roll. (The nearest scene safezone or road point, FindClosestSceneSafezone, is not ported: the
/// car's own place, the original's "none found".)
pub fn scene_frame(vehicles: &mut Vehicles, world: &World, zones: &[(Vec3, f32)]) -> Option<glam::Mat4> {
    let c = vehicles.player_mut()?;
    // (CineScript_FindClosestSceneSafezone: the place and heading replace the car's - Racer_InitializeResetPosition
    // and ResetWithDamageIntact put the car there upright; none found keeps the car's own)
    let (pos, f) = match closest_scene_safezone(world, zones, c.state.pos) {
        Some((p, h)) => {
            let h = h.to_radians();
            log::info!("police: scene place {:?} heading {:.1} (car at {:?})", p, h.to_degrees(), c.state.pos);
            (p, Vec3::new(h.sin(), 0.0, h.cos()))
        }
        None => (c.state.pos, c.state.forward()),
    };
    let q = glam::Quat::from_rotation_y((-f.x).atan2(-f.z));
    if let Some(vt) = c.params().veh.clone() {
        crate::physics::vehsim::set_pose(&mut c.state, &vt, &world.collider, [q * Vec3::X, q * Vec3::Y, q * Vec3::Z, pos], Vec3::ZERO, Vec3::ZERO);
    } else {
        c.state.rot = q;
        c.state.vel = Vec3::ZERO;
        c.state.ang_vel = Vec3::ZERO;
    }
    c.pose_history.reset();
    let (place, lean) = match world.collider.ground(pos, 5.0, 5.0) {
        Some(h) if h.normal.angle_between(Vec3::Y) > 0.002 => (Vec3::new(pos.x, h.point.y, pos.z), glam::Quat::from_rotation_arc(Vec3::Y, h.normal.normalize())),
        Some(h) => (Vec3::new(pos.x, h.point.y, pos.z), glam::Quat::IDENTITY),
        None => (pos, glam::Quat::IDENTITY),
    };
    Some(glam::Mat4::from_rotation_translation(lean * q, place))
}

/// The arrest safe zones (`tune/cutscene/sc_arrestsafe.zones`, else `arrestsafe.zones`): `ArrestSafeZones`,
/// `NumSafeZones n`, then n rows `id x y z heading` (degrees, the GetAngleY convention; the id unused).
pub fn load_safe_zones(data_dir: &Path) -> Vec<(Vec3, f32)> {
    let dir = data_dir.join("tune/cutscene");
    let text = ["sc_arrestsafe.zones", "0xf4ec4d9a", "arrestsafe.zones", "0xebaf318b"].iter().find_map(|f| std::fs::read_to_string(dir.join(f)).ok()).unwrap_or_default();
    let mut tok = text.split_whitespace();
    let mut out = Vec::new();
    if tok.by_ref().any(|t| t == "NumSafeZones") {
        let n: usize = tok.next().and_then(|t| t.parse().ok()).unwrap_or(0);
        for _ in 0..n {
            let v: Vec<f32> = tok.by_ref().take(5).filter_map(|t| t.parse().ok()).collect();
            if v.len() == 5 {
                out.push((Vec3::new(v[1], v[2], v[3]), v[4]));
            }
        }
    }
    out
}

/// CineScript_FindClosestSceneSafezone (F-23301 -> F-96161): A = the road
/// point (`scene_road_point`), B = the nearest arrest safe zone (radius 50 m doubling, 5 tries, F-83517; a zone within the
/// radius in 3D, the least x / z distance wins). Both: B when nearer (x / z, squared), else A within 10 m, else B
/// within 25 m, else A. (point, heading in degrees).
pub fn closest_scene_safezone(world: &World, zones: &[(Vec3, f32)], pos: Vec3) -> Option<(Vec3, f32)> {
    let d2 = |p: Vec3| (p.x - pos.x) * (p.x - pos.x) + (p.z - pos.z) * (p.z - pos.z);
    let mut b = None;
    let mut r = 50.0f32;
    for _ in 0..5 {
        let mut best = f32::INFINITY;
        for &(p, h) in zones {
            if (p - pos).length_squared() < r * r && d2(p) < best {
                best = d2(p);
                b = Some((p, h));
            }
        }
        if b.is_some() {
            break;
        }
        r *= 2.0;
    }
    let a = world.nav.as_ref().and_then(|nav| scene_road_point(nav, world, pos));
    match (a, b) {
        (Some(a), Some(b)) => Some(if d2(b.0) < d2(a.0) {
            b
        } else if d2(a.0) < 100.0 {
            a
        } else if d2(b.0) < 625.0 {
            b
        } else {
            a
        }),
        (a, b) => a.or(b),
    }
}

/// The road point of the scene safezone search (F-18669): the road of the nearest
/// lane within r = 5, 15 .. 245 m (curve point within r in 3D, by d^2 + dy^2), the road's nearest lane and its
/// direction there, of the road's lanes near that point (phase strictly inside, or within 16 m along) the kerb one
/// going the same way, its phase stepped by 0.1 towards the middle while a junction is within 10 m, the ground under
/// it (5 m up and down) and the heading against the lane's travel.
fn scene_road_point(nav: &NavSys, world: &World, pos: Vec3) -> Option<(Vec3, f32)> {
    use crate::vehicles::traffic::curve_nearest;
    let mut road = None;
    let mut r = 5.0f32;
    while r < 250.0 {
        let mut best = f32::MAX;
        for l in nav.lanes_near(pos, r, 1, 128) {
            let Some(c) = nav.curves.get(l) else { continue };
            let (_, d2, p) = curve_nearest(c, pos, if nav.lane_len(l) >= 450.0 { 8 } else { 4 }, 10);
            if !(d2 < r * r) {
                continue;
            }
            let Some(rd) = nav.lane_road(l) else { continue };
            let biased = d2 + (p.y - pos.y) * (p.y - pos.y);
            if biased < best {
                best = biased;
                road = Some(rd);
            }
        }
        if road.is_some() {
            break;
        }
        r += 10.0;
    }
    let lanes = nav.road_lanes(road?);
    let near: Vec<(usize, f32, f32, Vec3)> = lanes.iter().filter_map(|&l| nav.curves.get(l).map(|c| {
        let (t, d2, p) = curve_nearest(c, pos, 4, 10);
        (l, t, d2, p)
    })).collect();
    let mut first: Option<&(usize, f32, f32, Vec3)> = None;
    for n in &near {
        if first.map_or(true, |f| n.2 < f.2) {
            first = Some(n);
        }
    }
    let &(l0, t0, _, _) = first?;
    let dir = crate::aiv::norm_or(nav.curves[l0].eval_pair(t0).1, 1e-5, Vec3::X);
    let (mut lo, mut hi): (Option<(f32, usize, f32, Vec3)>, Option<(f32, usize, f32, Vec3)>) = (None, None);
    for &(l, t, _, p) in &near {
        let d = p - pos;
        let side = d.x * dir.z - d.z * dir.x;
        if !((t > 0.0 && t < 1.0) || d.dot(dir).abs() < 16.0) {
            continue;
        }
        if lo.map_or(true, |x| side < x.0) {
            lo = Some((side, l, t, p));
        }
        if hi.map_or(true, |x| side > x.0) {
            hi = Some((side, l, t, p));
        }
    }
    let (lo, hi) = (lo?, hi?);
    let s = |x: (f32, usize, f32, Vec3)| dir.dot(nav.curves[x.1].eval_pair(x.2).1);
    let (sl, sr) = (s(lo), s(hi));
    let pick = if sl >= 0.0 && sr < 0.0 {
        lo
    } else if sl < 0.0 && sr >= 0.0 {
        hi
    } else if sl >= 0.0 {
        if (hi.3 - lo.3).dot(Vec3::new(-dir.z, dir.y, dir.x)) > 0.0 { hi } else { lo }
    } else if (hi.3 - pos).length_squared() < (lo.3 - pos).length_squared() {
        hi
    } else {
        lo
    };
    let c = &nav.curves[pick.1];
    let mut t = pick.2;
    let step = if t < 0.5 { 0.1 } else { -0.1 };
    let mut p = c.eval_pair(t).0;
    while nav.node_within(Vec3::new(p.x, 0.0, p.z), 10.0, true).is_some() {
        let old = t;
        t = (t + step).clamp(0.0, 1.0);
        if t == 0.0 || t == 1.0 || (step > 0.0 && old <= 0.5 && t > 0.5) || (step < 0.0 && old >= 0.5 && t < 0.5) {
            break;
        }
        p = c.eval_pair(t).0;
    }
    let (p, d) = c.eval_pair(t);
    let hit = world.collider.ground(p, 5.0, 5.0)?;
    let h = (d.x.atan2(d.z) + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU).to_degrees();
    Some((hit.point, h))
}

/// The ticket scene after the officer's walk (CopPullover.sco 0x57f - 0x75b): a motorcycle's `cut_cop_fidget_bike`,
/// a car whose type name holds `_lr_` `sc_cut_cop_ticket_suv`, else `cut_cop_fidget_02`.
pub fn ticket_scene(car: &str, bike: bool) -> &'static str {
    if bike {
        "cut_cop_fidget_bike"
    } else if car.to_ascii_lowercase().contains("_lr_") {
        "sc_cut_cop_ticket_suv"
    } else {
        "cut_cop_fidget_02"
    }
}

#[cfg(test)]
mod scene_tests {
    use super::*;

    #[test]
    fn the_arrest_scene_goes_by_the_cops_of_the_chase() {
        // (CopArrest.sco: RAND_INT_RANGE's upper bound is included - two scenes for one cop, three for more)
        assert_eq!([0, 1].map(|r| arrest_scene(1, r)), ["cut_cop_arrest_07", "cut_cop_arrest"]);
        assert_eq!([0, 1, 2].map(|r| arrest_scene(3, r)), ["cut_cop_arrest_02", "cut_cop_arrest_03", "cut_cop_arrest_08"]);
        assert_eq!(arrest_hands_up("cut_cop_arrest"), None);
        assert_eq!(arrest_hands_up("cutscene/CUT_COP_ARREST_07"), Some((3.5, 1.5)));
    }

    #[test]
    fn the_ticket_scene_goes_by_the_car() {
        assert_eq!(ticket_scene("vp_dge_charger_69", false), "cut_cop_fidget_02");
        assert_eq!(ticket_scene("vp_cad_escalade_lr_06", false), "sc_cut_cop_ticket_suv");
        assert_eq!(ticket_scene("vp_duc_monster_09", true), "cut_cop_fidget_bike");
    }
}
