//! Police: law-breaking score, pull-over, pursuit, search and arrest.
//!
//! All numbers come from the game's own `tune/police/globals.xml`
//! (`mcPoliceTuningGlobals`); patrol cars start on the routes of
//! `tune/police/sc_patrolpath.xml`. The cruiser is `vp_chv_police_96`.

use crate::props::Inst;
use crate::render::Gpu;
use crate::vehicles::parked::Mwc;
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
        // (iLawFines is 15 words inside the globals block, all 100 from the constructor F-78390; the file fills the
        // first 14, so entry 14 - block traffic's - stays 100)
        if let Some(a) = arrays.get_mut("iLawFines") {
            while a.len() < 15 {
                a.push(100);
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
    /// a tactic on an AI racer ended by result 2 or 3 (escaped, a cop gone): the radio's REPORT_END_PURSUIT and the
    /// player's PLAYER_LOSE_POLICE (F-22530, not gated by the target); nothing of the player's record, reward or grace
    LostAi,
    /// the Dispatch to Location ended by result 2 or 3 (nobody at the place in fDispatchLocWaitTime, its cop gone): F-22530's
    /// tactic slot 0 stops the radio and says REPORT_SUSPECT_NOT_FOUND (F-23215, probability 1, no speech after it)
    DispatchLost,
    /// stopped for the cruiser: fine
    Ticket(i64),
    /// arrested during a pursuit: fine
    Busted(i64),
    /// the arrest's hold (fArrestStateTime) is over: the tactic succeeded and the manager posts COP_ARREST (F-22530),
    /// which runs CopArrest.sco - its scene
    ArrestScene,
}

/// The Pursuit's Delay state in the tactic's FSM update (F-95322): an ask of CatchUp is applied first (the Delay ends:
/// `hold` = 0, CatchUp entered and ticked in this update), else the tick F-50798 asks CatchUp when the state's time > the
/// hold, then the time + dt (F-88920). A hold of 0 is no Delay (its enter F-10824 asks CatchUp inside the begin).
fn delay_step(hold: &mut f32, t: &mut f32, ask: &mut bool, dt: f32) {
    if *hold <= 0.0 {
        return;
    }
    if *ask {
        (*hold, *t, *ask) = (0.0, 0.0, false);
        return;
    }
    if *t > *hold {
        *ask = true;
    }
    *t += dt;
}

/// The AlongSide's side pick F-14848: true = the target's right. `a` / `b`: the points fAlongSideDist to the target's right /
/// left over the nav mesh with the cop brain's filter (F-72689 with brain +164 = the police brain's mcCopFilter, which allows
/// every material: `on_mesh`); `c` / `d`: the same points over a road (F-90092); `on_right`: the target on the cop's right
/// (F-30098). One point over the mesh: that side; both: the road decides, the right first; else the side away from the cop.
fn side_pick(a: bool, b: bool, c: bool, d: bool, on_right: bool) -> bool {
    match (a, b) {
        (true, false) => true,
        (false, true) => false,
        (true, true) if c => true,
        (true, true) if d => false,
        _ => !on_right,
    }
}

/// The tactics' closing-in test F-32902: B (the cop) heads at A (the target). With `r` != 0 the squared distance (dy dy, dz
/// and dx fused) must be under r^2; B's chassis speed `sb` (+224) above fMovingSpeed x 0.447 and above A's `sa`; then the
/// unit of A - B (F-41334, eps 1e-5, fallback (1, 0, 0)) dotted with B's velocity / `sb` over 0.9. The vector unit's
/// subtract and multiply flush denormals.
fn closing_in(a: Vec3, b: Vec3, sa: f32, sb: f32, vb: Vec3, r: f32, moving_speed: f32) -> bool {
    if r != 0.0 {
        let d2 = fmas(a.x - b.x, a.x - b.x, fmas(a.z - b.z, a.z - b.z, (a.y - b.y) * (a.y - b.y)));
        if d2 >= r * r {
            return false;
        }
    }
    if sb <= moving_speed * 0.447 || sb <= sa {
        return false;
    }
    let ftz = |x: f32| if x.abs() < f32::MIN_POSITIVE { 0.0 * x } else { x };
    let d = Vec3::new(ftz(ftz(a.x) - ftz(b.x)), ftz(ftz(a.y) - ftz(b.y)), ftz(ftz(a.z) - ftz(b.z)));
    let k = ftz(1.0 / sb);
    let v = Vec3::new(ftz(ftz(vb.x) * k), ftz(ftz(vb.y) * k), ftz(ftz(vb.z) * k));
    let n = crate::aiv::norm_or(d, 1e-5, Vec3::X);
    fmas(n.x, v.x, fmas(n.y, v.y, n.z * v.z)) > 0.9
}

/// The police manager's detecting-cop pick F-76033 (PoliceManager_CanAnyPoliceDetectTarget's body; the law update F-52240
/// asks it with need_check, need_free and group 1): the first of the manager's `n` units, in its order, that is there
/// (`unit(i)` = Some((+1072 may check law breaking, in a tactic F-72241, +872 group))), may check when `need_check`, is in no
/// tactic when `need_free`, is of `group`, and then sees the target (`sees(i)`, F-94676, asked only for such a unit).
fn first_detecting(n: usize, need_check: bool, need_free: bool, group: u8, unit: impl Fn(usize) -> Option<(bool, bool, u8)>, mut sees: impl FnMut(usize) -> bool) -> Option<usize> {
    (0..n).find(|&i| match unit(i) {
        Some((check, tactic, g)) => (!need_check || check) && (!need_free || !tactic) && g == group && sees(i),
        None => false,
    })
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
    /// the plan's countdown (+588), its two goals and the speed to arrive with (-1: full pace)
    replan: f32,
    goals: [Vec3; 2],
    speed: f32,
    plan_fwd: Vec3,
    plan_speed: f32,
    /// brain +640: the target's position, taken by the brain update (F-74140) while it is on the nav mesh or has a
    /// teleport target there (F-72689 / F-45363); the quick re-aim's goal
    last_seen: Vec3,
    head_on: bool,
    /// Block's steer, taken at its enter (F-56982, +700): full lock towards the side the target was on
    block_steer: f32,
    /// pursuit: the machine started (its Delay asks CatchUp), the state asked for (entered at the next update), Halt's
    /// steer away (+16, taken once the cop closes in), and the brain's command state (+568) and handbrake mode (+688)
    begun: bool,
    next: Option<Tac>,
    halt_steer: f32,
    cmd: u8,
    hb: bool,
    /// pull-over Halt: the squared distance at its last update (+16)
    halt_d2: f32,
    /// a command sent this update (F-24671 -> F-83687, applied after the machines) and the brain's fields it wrote
    sent: bool,
    brain: BrainCmd,
}

/// What the pursuit's ticks read in one update: the brain's predicates (F-74140's helpers) - within fSafeDist, within 1.5 x
/// fAlongSideDist, within 5 m, the target moving, going the same way, ahead, on the cop's right, facing the same way, every
/// cop within half fSafeDist, closing in -, the side F-14848 picks, the AlongSide's re-pick gate (tactic +36 none or its
/// +56 zero), the bearing off the cop's right axis and the target's speed (m/s).
#[derive(Clone, Copy, Debug, Default)]
struct PursuitSees {
    near_safe: bool,
    near_along: bool,
    near5: bool,
    moving: bool,
    same_dir: bool,
    ahead: bool,
    on_right: bool,
    same_facing: bool,
    all_block: bool,
    closing: bool,
    pick: bool,
    gate: bool,
    bearing: f32,
    speed: f32,
}

/// The Pursuit's machine for its cop, one update (F-68032's CatchUp, AlongSide, Block, Halt; Delay, Search and Arrest are
/// the port's `flee_hold` and heat). The tactic's update first asks Halt when the target lost over 4 m/s since the last
/// update within 5 m (not in Halt or Block, none asked yet; `last_speed`: the speed the base update kept, +64); then the
/// FSM (F-95322): the asked state entered (F-43677; every ask here passes the table F-64295), the state's tick, its time
/// + dt (F-88920). Enters (F-13059, F-97171, F-56982, F-29519): CatchUp Pursue (4); AlongSide the side picked, its timers 0 / 1 s, AlongSide (5); Block Stop
/// in handbrake mode, full lock to the target's side; Halt Stop when within fSafeDist, its steer 0. Ticks: CatchUp
/// (F-88702) - within fSafeDist: Halt if the target stands, AlongSide after 3 s going the same way, ahead, 75 - 105 deg;
/// AlongSide (F-85137) - out of reach 10 s: CatchUp (and nothing else); within: the side timer down, at < 0 with the gate
/// the side again (AlongSide resent if it changed) and 1 s, then Block when facing the same way, the target on the other
/// hand and not ahead; Block (F-85039) - CatchUp unless every cop within half fSafeDist; Halt (F-84018) - its steer away
/// once closing in, within fSafeDist and under fArrestTgtThresholdSpd (`thresh`, mph x 2.237) the Stop kept (resent
/// unless in it without handbrake), else CatchUp.
fn pursuit_machine(tac: &mut CopTac, s: &PursuitSees, thresh: f32, last_speed: f32, dt: f32) {
    if !tac.begun {
        tac.begun = true;
        tac.next = Some(Tac::CatchUp);
    } else if tac.next.is_none() && matches!(tac.st, Tac::CatchUp | Tac::AlongSide) && last_speed - s.speed > 4.0 && s.near5 {
        tac.next = Some(Tac::Halt);
    }
    if let Some(st) = tac.next.take() {
        tac.st = st;
        tac.t = 0.0;
        match st {
            Tac::CatchUp => (tac.cmd, tac.sent) = (4, true),
            Tac::AlongSide => {
                (tac.far_t, tac.side_t, tac.side, tac.cmd, tac.sent) = (0.0, 1.0, s.pick, 5, true);
            }
            Tac::Block => {
                (tac.cmd, tac.hb, tac.block_steer, tac.sent) = (1, true, if s.on_right { 1.0 } else { -1.0 }, true);
            }
            Tac::Halt => {
                if s.near_safe {
                    (tac.cmd, tac.hb, tac.sent) = (1, false, true);
                }
                tac.halt_steer = 0.0;
            }
            Tac::Avoid => {}
        }
    }
    match tac.st {
        Tac::CatchUp if s.near_safe => {
            if !s.moving {
                tac.next = Some(Tac::Halt);
            } else if tac.t > 3.0 && s.same_dir && s.ahead && s.bearing > 75.0 && s.bearing < 105.0 {
                tac.next = Some(Tac::AlongSide);
            }
        }
        Tac::AlongSide => {
            if !s.near_along {
                tac.far_t += dt;
                if tac.far_t > 10.0 {
                    tac.next = Some(Tac::CatchUp);
                }
            } else {
                tac.side_t -= dt;
                tac.far_t = 0.0;
                if tac.side_t < 0.0 && s.gate {
                    if s.pick != tac.side {
                        (tac.side, tac.cmd, tac.sent) = (s.pick, 5, true);
                    }
                    tac.side_t = 1.0;
                }
                if s.same_facing && tac.side != s.on_right && !s.ahead {
                    tac.next = Some(Tac::Block);
                }
            }
        }
        Tac::Block if !s.all_block => tac.next = Some(Tac::CatchUp),
        Tac::Halt => {
            if tac.halt_steer == 0.0 && s.closing {
                tac.halt_steer = if s.on_right { -1.0 } else { 1.0 };
            }
            if s.near_safe && s.speed * 2.237 < thresh {
                if !(tac.cmd == 1 && !tac.hb) {
                    (tac.cmd, tac.hb, tac.sent) = (1, false, true);
                }
            } else {
                tac.next = Some(Tac::CatchUp);
            }
        }
        _ => {}
    }
    tac.t += dt;
}

/// What the pull-over's update and ticks read in one update: every cop within fPullOverDist - 1 (F-90575), the target
/// moving (F-96080 / F-43975), closing in (F-32902) with no range / within 7 m / 50 m, within 50 m / fPullOverDist
/// (F-29545), the side F-14848 picks, the re-pick gate (tactic +36 none or its +56 zero), the cop's speed (F-81535) and the
/// squared distance cop - target (dy, dz, dx, fused: F-38149 / F-82776).
#[derive(Clone, Copy, Debug, Default)]
struct PullSees {
    all_halt: bool,
    moving: bool,
    closing0: bool,
    closing7: bool,
    closing50: bool,
    near50: bool,
    near_pull: bool,
    pick: bool,
    gate: bool,
    cspeed: f32,
    d2: f32,
}

/// The pull-over's machine for its cop, one update (F-36876's CatchUp, AvoidCollision, Halt; Ticket is the port's heat);
/// returns the factor the cop's velocity is scaled by. The begin (F-74547) enters CatchUp. The update asks Halt when every
/// cop is within fPullOverDist - 1, the target stands and the cop is under 4.47 m/s or not closing in (not in Halt, none
/// asked); then the FSM (F-95322; table F-20341): the asked state entered, the tick, the time + dt. Enters: CatchUp (F-50661)
/// Pull Over (3); AvoidCollision (F-69591) the side picked, AlongSide (5) with it, its timer 1 s, the cop's velocity x 0.7;
/// Halt (F-38149) Stop without handbrake, its growing time 0, the squared distance kept. Ticks: CatchUp (F-91234) - after
/// 0.5 s and closing in within 50 m: AvoidCollision; AvoidCollision (F-18622) - the timer down, at < 0 with the gate the side
/// again (AlongSide resent if it changed) and 1 s; after 0.5 s and not within 50 m: CatchUp; Halt (F-82776) - the gap growing while the
/// target moves for > 0.5 s, or not within fPullOverDist: CatchUp. After the FSM: the target standing and the cop closing in
/// within 7 m - its velocity x 0.5.
fn pullover_machine(tac: &mut CopTac, s: &PullSees, dt: f32) -> f32 {
    let mut factor = 1.0f32;
    if !tac.begun {
        (tac.begun, tac.st, tac.t, tac.cmd, tac.sent) = (true, Tac::CatchUp, 0.0, 3, true);
    }
    if tac.st != Tac::Halt && tac.next.is_none() && s.all_halt && !s.moving && (s.cspeed < 4.47 || !s.closing0) {
        tac.next = Some(Tac::Halt);
    }
    if let Some(st) = tac.next.take() {
        tac.st = st;
        tac.t = 0.0;
        match st {
            Tac::CatchUp => (tac.cmd, tac.sent) = (3, true),
            Tac::Avoid => {
                (tac.side, tac.cmd, tac.replan, tac.side_t, tac.sent) = (s.pick, 5, 0.0, 1.0, true);
                factor *= 0.7;
            }
            Tac::Halt => (tac.cmd, tac.hb, tac.grow_t, tac.halt_d2, tac.sent) = (1, false, 0.0, s.d2, true),
            _ => {}
        }
    }
    match tac.st {
        Tac::CatchUp if tac.t > 0.5 && s.closing50 => tac.next = Some(Tac::Avoid),
        Tac::Avoid => {
            tac.side_t -= dt;
            if tac.side_t < 0.0 && s.gate {
                if s.pick != tac.side {
                    (tac.side, tac.cmd, tac.sent) = (s.pick, 5, true);
                }
                tac.side_t = 1.0;
            }
            if tac.t > 0.5 && !s.near50 {
                tac.next = Some(Tac::CatchUp);
            }
        }
        Tac::Halt => {
            tac.grow_t = if s.moving && s.d2 > tac.halt_d2 { tac.grow_t + dt } else { 0.0 };
            tac.halt_d2 = s.d2;
            if tac.grow_t > 0.5 || !s.near_pull {
                tac.next = Some(Tac::CatchUp);
            }
        }
        _ => {}
    }
    tac.t += dt;
    if !s.moving && s.closing7 {
        factor *= 0.5;
    }
    factor
}

/// What the brain states' per-tick overrides read: the target moving (F-43975), ahead of the cop (F-99390), going the same
/// way (F-80249), the brain's race flag (F-42951), the cop's and the target's speed (F-81535, F-41260), the squared
/// distance cop - target (dy*dy, then dz, dx fused) and fSafeDist.
#[derive(Clone, Copy, Debug, Default)]
struct BrainSees {
    moving: bool,
    ahead: bool,
    same_dir: bool,
    ed78: bool,
    cspeed: f32,
    tspeed: f32,
    d2: f32,
    safe: f32,
}

/// The cop brain's fields the brain command F-83687 writes (the command dispatcher F-24671 hands it every command but 2).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct BrainCmd {
    /// +568 the command, +572 / +580 / +584 its timers, +576 a count, +588 the plan countdown (command 0 only)
    cmd: u32,
    t572: f32,
    n576: u32,
    t580: f32,
    t584: f32,
    t588: f32,
    b592: u8,
    b593: u8,
    /// +596 the unseen time, +600 the time in the search cone (the port's `Eye`)
    unseen: f32,
    cone_t: f32,
    b604: u8,
    b605: u8,
    /// +676 the block's tactic, +680 / +684 its two targets
    tactic: u32,
    tgt: [u32; 2],
    /// +688..+700 Stop (1): handbrake mode, amount, throttle, steer
    stop: [u32; 4],
    /// +704 the block's +12 clamped to 0..1
    scale: f32,
    /// +708 / +712 AlongSide (5): the side, its distance
    side: u8,
    side_d: f32,
    /// +716 AlignWithTarget (6), +720 Respond (8): the place, +736 / +740 Search (7): its side and angle
    align: f32,
    place: [u32; 4],
    srch_side: u8,
    srch_ang: f32,
    b772: u8,
    /// the racing brain's final speed (+292, F-39608)
    final_speed: f32,
}

/// The brain command F-83687, one call: command 0 - the plan countdown 0, the route cleared (F-69438), no targets, +592 /
/// +593 0; commands 1..8 - the block's targets (+0 / +4), +704 = its +12 clamped to 0..1 (by `fsel`), the final speed -1,
/// and Stop's (1) byte +16 and floats +20..+28, AlongSide's (5) byte +16 and +20, AlignWithTarget's (6) +16, Search's (7)
/// byte +16 and +20, Respond's (8) vector +16..+31. Then for every command: the timers 0, the command stored, **the unseen
/// and cone times 0 unless it is Search (7)**, +604 / +605 0, +676 the block's +8 (0 without a block), +772 0; +592 0 when
/// the previous command was 0 and this is not. Returns (the route cleared, the machine's reset asked - the command changed,
/// not from 0 to 0, and the machine's state not 0 -, the state asked: the command).
fn brain_command(b: &mut BrainCmd, cmd: u32, blk: Option<&[u32; 8]>, machine_state: i32) -> (bool, bool, u32) {
    let w = |i: usize| blk.map_or(0, |k| k[i]);
    let f = |i: usize| f32::from_bits(w(i));
    let byte = |i: usize| (w(i) >> 24) as u8;
    let mut route = false;
    if cmd == 0 {
        b.t588 = 0.0;
        route = true;
        b.tgt = [0, 0];
        (b.b592, b.b593) = (0, 0);
    } else if (1..9).contains(&(cmd as i32)) {
        b.tgt = [w(0), w(1)];
        let lo = if f(3) >= 0.0 { f(3) } else { 0.0 };
        b.scale = if lo - 1.0 >= 0.0 { 1.0 } else { lo };
        b.final_speed = -1.0;
        match cmd {
            1 => b.stop = [(b.stop[0] & 0x00FF_FFFF) | ((byte(4) as u32) << 24), w(5), w(6), w(7)],
            5 => (b.side, b.side_d) = (byte(4), f(5)),
            6 => b.align = f(4),
            7 => (b.srch_side, b.srch_ang) = (byte(4), f(5)),
            8 => b.place = [w(4), w(5), w(6), w(7)],
            _ => {}
        }
    }
    let prev = b.cmd;
    (b.t572, b.t580, b.cmd, b.t584, b.n576) = (0.0, 0.0, cmd, 0.0, 0);
    if cmd != 7 {
        (b.cone_t, b.unseen) = (0.0, 0.0);
    }
    (b.b604, b.b605) = (0, 0);
    b.tactic = w(2);
    b.b772 = 0;
    let mut reset = false;
    if !(prev == 0 && cmd == 0) {
        if prev == 0 {
            b.b592 = 0;
        }
        reset = prev != cmd && machine_state != 0;
    }
    (route, reset, cmd)
}

/// A Pursuit's / Double Pursuit's may-escalate (vt+24; the Double Pursuit's F-73000): the radio's busy byte F-94735 clear, then
/// the base test F-38605 - the may-escalate byte +28, not in Search (vt+40: the Double Pursuit's F-12623, its machine's state
/// == 2), the arrest timer +108 not greater than +0.0 (fcmpu: NaN passes), not in Arrest (vt+36: F-48174, state == 1). The
/// port's Search / Arrest are `Heat::Search` / `Heat::Busted`; the Pursuit's Delay test is the caller's.
fn tactic_may_escalate(radio_busy: bool, may: bool, searching: bool, arresting: bool, arrest: f32) -> bool {
    !radio_busy && may && !searching && !(f64::from(arrest) > 0.0) && !arresting
}

/// What StartTactic (F-33550) does with the tactic already on the target (its type and may-escalate byte +28), for an asked
/// type: the same type - that tactic is returned as it is; a lower type whose byte is 0 - refused (nothing starts); else the
/// old one ends (vt+12(0)) and the new one starts, taking the old one's duration +104 when the old type is above 1 and the new
/// type above the old (a Pursuit's time goes on into the Double Pursuit and the Arrow Trap; a pull-over's does not).
#[derive(Clone, Copy, Debug, PartialEq)]
enum StartRule {
    Keep,
    Refuse,
    Start { end_old: bool, carry: bool },
}

fn start_rule(kind: i32, old: Option<(i32, bool)>) -> StartRule {
    match old {
        None => StartRule::Start { end_old: false, carry: false },
        Some((t, _)) if t == kind => StartRule::Keep,
        Some((t, b28)) if t < kind && !b28 => StartRule::Refuse,
        Some((t, _)) => StartRule::Start { end_old: true, carry: t > 1 && kind > t },
    }
}

/// The Search state's clock test F-73289 (its vt+80, read by the tactics' vt+44 F-77220 as the target lost): the clock widened to
/// double and compared greater than +0.0 by fcmpu - false for -0.0, +0.0, the negatives and NaN.
fn search_clock_runs(clock: f32) -> bool {
    f64::from(clock) > 0.0
}

/// The Search command's (7) block: F-60530 (+0..+8 0, +12 1.0, byte +16 0, +20 0.0), then the sender's byte +16 (search from the
/// cop's own place with a random turn at every junction: the dispatch's searcher) and +20 (the turn at the first junction).
/// Respond's block (command 8): F-11391 (+0..+8 0, +12 1.0, +16..+31 a global vector, zero in the image), then the sender
/// (Respond's enter F-81858) stores the targets and the tactic (+0 / +4 / +8; the port's brains read none), +12 1.0 again
/// and +16..+31 the place: the tactic's +176 when its y is not below -500, else the target's position. The dispatch's
/// request (F-15449, -1000 x 3 at +32) gets the target's position from its one caller (no place given), so +176 is where the
/// player was. (+28, the place's fourth word, is not read by any command: 0 here)
fn respond_block(place: Vec3) -> [u32; 8] {
    let mut b = [0u32; 8];
    b[3] = 1.0f32.to_bits();
    (b[4], b[5], b[6]) = (place.x.to_bits(), place.y.to_bits(), place.z.to_bits());
    b
}

fn search_block(random: bool, angle: f32) -> [u32; 8] {
    let mut b = [0u32; 8];
    b[3] = 1.0f32.to_bits();
    b[4] = (random as u32) << 24;
    b[5] = angle.to_bits();
    b
}

/// A command's block as the port's senders build it: Stop (1) F-72181 (+12 1.0, +16 0, +20 1.0, +24 -1, +28 -1000) - Block's
/// enter F-56982 sets the handbrake byte, +24 0 and its steer -, AlongSide (5) F-94949 (+12 1.0, +16 the side, +20
/// fAlongSideDist), the others F-59773 (+12 1.0). No targets or tactic (the port's brains read none of them).
fn cmd_block(cmd: u8, hb: bool, block_steer: f32, side: bool, along: f32) -> [u32; 8] {
    let mut b = [0u32; 8];
    b[3] = 1.0f32.to_bits();
    match cmd {
        1 if hb => (b[4], b[5], b[6], b[7]) = (1 << 24, 1.0f32.to_bits(), 0.0f32.to_bits(), block_steer.to_bits()),
        1 => (b[5], b[6], b[7]) = (1.0f32.to_bits(), (-1.0f32).to_bits(), (-1000.0f32).to_bits()),
        5 => (b[4], b[5]) = ((side as u32) << 24, along.to_bits()),
        _ => {}
    }
    b
}

/// PoliceManager_IsAnyPoliceCloseToPlayer's body F-91297: with a player (`player`; none without the player manager or its
/// racer), any of the manager's active units on the surface (mode +1152 0, not underground +880; in the port the cop cars on
/// the road) whose squared distance to the player (dy dy, then dz and dx fused) is strictly under r x r.
pub(crate) fn any_police_close(player: Option<Vec3>, units: impl IntoIterator<Item = Vec3>, r: f32) -> bool {
    let Some(p) = player else { return false };
    let r2 = r * r;
    units.into_iter().any(|u| {
        let (dy, dz, dx) = (u.y - p.y, u.z - p.z, u.x - p.x);
        fmas(dx, dx, fmas(dz, dz, dy * dy)) < r2
    })
}

/// The police's burn-out test F-25339 (the law update's offence and the tactic's target): the car's burnout charge
/// (chassis +580, `VehState::burn`) fcmpu-greater than 0 (NaN and -0 are not; a subnormal is). No car / chassis: false.
pub(crate) fn burn_charged(burn: Option<f32>) -> bool {
    burn.map_or(false, |b| b > 0.0)
}

/// Racer_IsBurningOut F-48805: the burnout state (chassis +578, `VehState::burn_state`) as a signed byte greater than 0.
/// No racer / car: false (the original does not test the chassis).
pub(crate) fn burn_state_on(state: Option<u8>) -> bool {
    state.map_or(false, |s| (s as i8) > 0)
}

/// The patrol's path change F-84839 for a unit whose brain is the patrol's and that has a group: `group` = the group's paths,
/// `own` = the unit's path in it (+1124), `timer` = +1128. A group of one path - every group of the disc's patrol file -
/// changes nothing and draws no number. Else, with the timer <= 0 (NaN passes): one step of the shared MWC `rng`, its low 23
/// bits x 2^-23 against `pct` (iChangePathProbPrcnt, an int) x 0.01; above it the timer becomes 1 s and nothing changes;
/// at or under it the nearest point (dy dy, dz, dx fused; taken unless d >= the best so far, from 9999979520) over the
/// group's other paths: Some((path, point)), (-1, -1) when there is none.
fn path_change(group: &[Vec<Vec3>], own: i32, timer: &mut f32, pos: Vec3, pct: i32, rng: &mut (u32, u32)) -> Option<(i32, i32)> {
    if group.len() == 1 || *timer > 0.0 {
        return None;
    }
    let x = rng.0 as u64 * 0x5CDC_FAA7 + rng.1 as u64;
    *rng = (x as u32, (x >> 32) as u32);
    let r = ((x as u32) & 0x7F_FFFF) as f32 * f32::from_bits(872415232);
    let p = pct as f32 * f32::from_bits(1008981770);
    if !(r <= p) {
        *timer = 1.0;
        return None;
    }
    let (mut best, mut out) = (f32::from_bits(1343554277), (-1, -1));
    for (j, path) in group.iter().enumerate() {
        if j as i32 == own {
            continue;
        }
        for (k, q) in path.iter().enumerate() {
            let (dy, dz, dx) = (pos.y - q.y, pos.z - q.z, pos.x - q.x);
            let d = fmas(dx, dx, fmas(dz, dz, dy * dy));
            if !(d >= best) {
                (best, out) = (d, (j as i32, k as i32));
            }
        }
    }
    Some(out)
}

/// The cop brain's Idle command handler F-44966 (vt+84; the brain command F-83687 reaches it through F-17363 after resetting the
/// machine to Idle): the state it asks the machine for (SetNextState F-28114) for the brain's command, and whether it clears
/// the machine's pending state (+16 = -1, command 0). 1 Stop -> 3 Stopping, 3 -> 1 PullOver, 4 -> 2 Pursuit, 5 -> 4 AlongSide,
/// 6 -> 5 LeadTarget, 7 -> 7 Search, 8 -> 8 GoToPosition; 2 (Follow) and anything above 8 (unsigned) ask nothing. The ask is
/// always made from Idle, whose row of the transition table F-77775 allows every state, so it always lands.
fn idle_ask(cmd: u32) -> (Option<u32>, bool) {
    match cmd {
        0 => (None, true),
        1 => (Some(3), false),
        3 => (Some(1), false),
        4 => (Some(2), false),
        5 => (Some(4), false),
        6 => (Some(5), false),
        7 => (Some(7), false),
        8 => (Some(8), false),
        _ => (None, false),
    }
}

/// The Stop command's parameters on the brain: +688 handbrake mode, +692 the amount, +696 the throttle (taken when >= 0),
/// +700 the steer (-1000: none).
#[derive(Clone, Copy, Debug)]
struct StopParams {
    hb: bool,
    amount: f32,
    throttle: f32,
    steer: f32,
}

/// The overrides the cop's brain state puts over the driver's input each tick (vt slot 15), by the state its command +568 asks
/// (`idle_ask`: 1 -> 3 Stopping, 4 -> 2 Pursuit, 5 -> 4 AlongSide; the others' ticks put none); `side_d` is the AlongSide's distance (+712). Returns throttle
/// (+744), brake (+748), handbrake (+752), steer (+756), as the setters F-50239 / F-39048 / F-10965 clamp them to 0..1
/// (steer -1..1); -1 (steer -1000) is no override. Pursuit (F-68176): brake 1 when the target stands within fSafeDist, or
/// it is ahead going the same way within 20 m and the cop is faster. AlongSide (F-33668): moving - the same way, not ahead,
/// the cop faster and (no race flag or beyond 1.5 x the side distance): brake 1; standing - within the side distance + 1:
/// brake 1. Stopping (F-87827, its tick F-10174 while the brain has a tactic): handbrake mode - the amount as handbrake,
/// the throttle when >= 0; else the amount as brake; the steer unless -1000.
fn brain_override(cmd: u8, side_d: f32, stop: &StopParams, s: &BrainSees) -> [f32; 4] {
    // (fsel: NaN and below 0 -> 0, above 1 -> 1; the steer -1..1)
    let c01 = |v: f32| {
        let a = if v >= 0.0 { v } else { 0.0 };
        if a - 1.0 >= 0.0 { 1.0 } else { a }
    };
    let cs = |v: f32| {
        let a = if v - -1.0 >= 0.0 { v } else { -1.0 };
        if a - 1.0 >= 0.0 { 1.0 } else { a }
    };
    let mut o = [-1.0, -1.0, -1.0, -1000.0];
    match idle_ask(cmd as u32).0 {
        Some(2) => {
            if (!s.moving && s.d2 < s.safe * s.safe) || (s.ahead && s.same_dir && s.d2 < 400.0 && s.cspeed > s.tspeed) {
                o[1] = c01(1.0);
            }
        }
        Some(4) => {
            let brake = if s.moving {
                s.same_dir && !s.ahead && s.cspeed > s.tspeed && (!s.ed78 || { let r = side_d * 1.5; s.d2 > r * r })
            } else {
                let r = side_d + 1.0;
                s.d2 < r * r
            };
            if brake {
                o[1] = c01(1.0);
            }
        }
        Some(3) => {
            if stop.hb {
                o[2] = c01(stop.amount);
                if stop.throttle >= 0.0 {
                    o[0] = c01(stop.throttle);
                }
            } else {
                o[1] = c01(stop.amount);
            }
            if stop.steer != -1000.0 {
                o[3] = cs(stop.steer);
            }
        }
        _ => {}
    }
    o
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

/// The squad tactics' transition tests (slot 12 of their machines): the Arrow Trap's F-69937 (0 CatchUp, 1 Arrest, 2 Halt,
/// 3 Search) - CatchUp to any other, nothing out of Arrest, Halt to anything, Search to the first three; the Double
/// Pursuit's F-46230 (0 AlongSide, 1 Arrest, 2 Search) - AlongSide to Arrest or Search, Search to AlongSide or Arrest.
fn squad_may_go(arrow: bool, from: u8, to: i32) -> bool {
    match (arrow, from) {
        (true, 0) => (1..=3).contains(&to),
        (true, 2) => true,
        (true, 3) => (0..=2).contains(&to),
        (false, 0) => (1..=2).contains(&to),
        (false, 2) => (0..=1).contains(&to),
        _ => false,
    }
}

/// The squad tactics' CatchUp (the Double Pursuit's AlongSide) / Halt machine, one update (F-95322): the state asked for in
/// the last update is entered first (F-43677), then the state's update runs. CatchUp's enter (F-81752: slots -1, hysteresis
/// 2, the formation at once, the swipe timer and latch 0, lock 5 s) is followed in the same update by its update (F-47983 /
/// F-41674): the lock counts down, the formation once it has run out (the lock again on a send), the swipes, then the Arrow
/// Trap asks for Halt with a cop within fSafeDist of a target that is not moving. Halt (F-16041) asks for CatchUp with no
/// cop within fSafeDist or the target over fArrestTgtThresholdSpd. Returns true when Halt's update ran (its cops' stops
/// are the caller's).
#[allow(clippy::too_many_arguments)]
fn squad_machine<C>(sq: &mut Squad, cx: &mut C, arrow: bool, any_safe: bool, t_moving: bool, fast: bool, dt: f32,
                    formation: &mut dyn FnMut(&mut Squad, &mut C, f32, bool) -> bool, swipes: &mut dyn FnMut(&mut Squad, &mut C)) -> bool {
    if let Some(h) = sq.next.take() {
        sq.halt = h;
        if !h {
            sq.begun = false;
        }
    }
    if sq.halt {
        if (!any_safe || fast) && squad_may_go(arrow, 2, 0) {
            sq.next = Some(false);
        }
        return true;
    }
    if !sq.begun {
        let next = sq.next;
        *sq = Squad { begun: true, hyst: 2.0, next, ..Default::default() };
        formation(sq, cx, 0.0, true);
        sq.lock = 5.0;
    }
    sq.lock -= dt;
    if sq.lock < 0.0 && formation(sq, cx, dt, false) {
        sq.lock = 5.0;
    }
    swipes(sq, cx);
    if arrow && any_safe && !t_moving && squad_may_go(arrow, 0, 2) {
        sq.next = Some(true);
    }
    false
}

/// One cop brain's tests against its target as the squad's swipes read them (F-47983 / F-41674), in the original's float
/// order. `pos` / `right` / `back` / `vel`: the cop car's matrix rows 3 / 0 / 2 and velocity, `tpos` / `tvel` the target's.
/// `dsq`: the squared distance (F-39842: y y, then z, then x fused; within r when below r x r, F-29545); `ahead`: the third
/// row against the target below 0 (F-53459 / F-99390); `same_dir`: the two velocities above 0 (F-80249); `on_right`: the
/// first row against the target above 0 (F-57435 / F-30098); `bearing`: F-88594 of the first row and the target, x 57.2957...
/// in single (F-90304; its cache at brain +760 / +764 is cleared by the brain update F-74140).
#[derive(Clone, Copy, Debug)]
struct SwipeGeo {
    dsq: f32,
    ahead: bool,
    same_dir: bool,
    on_right: bool,
    bearing: f32,
}

fn swipe_geo(pos: Vec3, right: Vec3, back: Vec3, vel: Vec3, tpos: Vec3, tvel: Vec3) -> SwipeGeo {
    use crate::physics::exact::{dot_xyz, dot_xzy};
    let to = tpos - pos;
    SwipeGeo {
        dsq: arrest_d2(pos, tpos),
        ahead: dot_xyz(back, to) < 0.0,
        same_dir: dot_xzy(vel, tvel) > 0.0,
        on_right: on_side(right, pos, tpos),
        bearing: crate::aiv::ang3(right, to) * f32::from_bits(0x4265_2EE0),
    }
}

/// The Pursuit's tests for its cop, one update, with the helpers its ticks call (F-68032, CatchUp F-88702, AlongSide F-85137,
/// Block F-85039, Halt F-84018): `g` the cop's `swipe_geo`; within fSafeDist / 1.5 x fAlongSideDist (single product) / 5 m on
/// the squared distance (F-29545); every cop within half fSafeDist (F-90575 on the same test); facing the same way on the
/// frames' third rows (F-43223); the target moving (F-43975) on its speed `tspeed` (the chassis' +224, F-41260); `closing`
/// (F-32902, `closing_in`) and the side `pick` (F-14848) as the caller has them.
#[allow(clippy::too_many_arguments)]
fn pursuit_sees(g: &SwipeGeo, all_d2: &[f32], safe: f32, along: f32, cback: Vec3, tback: Vec3, tspeed: f32, moving_speed: f32,
                closing: bool, pick: bool) -> PursuitSees {
    let ra = along * 1.5;
    let rb = 0.5 * safe;
    PursuitSees {
        near_safe: g.dsq < safe * safe,
        near_along: g.dsq < ra * ra,
        near5: g.dsq < 25.0,
        moving: moving(tspeed, moving_speed),
        same_dir: g.same_dir,
        ahead: g.ahead,
        on_right: g.on_right,
        same_facing: same_facing(tback, cback),
        all_block: all_d2.iter().all(|&d| d < rb * rb),
        closing,
        pick,
        gate: true,
        bearing: g.bearing,
        speed: tspeed,
    }
}

/// The swipes of one CatchUp / AlongSide update (F-47983 / F-41674, after the lock and the formation), on the tactic's cops'
/// tests `g` (`a` / `b`: the right / left flank's index, slots +164 / +168) and their brains' head-on flags (+774, F-48012).
/// The right flank: within 10 m (the Double Pursuit's 15), the same way, the Arrow Trap's not ahead, not on its right,
/// bearing 135 - 160 deg (160 - 180): the latch (+32) set, on setting it the line (returned true) and the Arrow Trap's wait
/// +24 = 0.5 s; full lock left and full brake while the wait is 0; outside the window wait and latch 0. The left flank: on its
/// right, 20 - 45 deg (0 - 20), full lock right and full brake (its wait +28 is never set). The wait then counts down to 0.
/// Then every cop whose head-on flag is set within `head_on_dist`: the handbrake, full lock to the target's side (keeping a
/// swipe's brake). Writes `ovr[k]` (steer F-77791, brake F-39048, handbrake F-10965).
#[allow(clippy::too_many_arguments)]
fn squad_swipes(sq: &mut Squad, arrow: bool, dt: f32, a: Option<usize>, b: Option<usize>, g: &[Option<SwipeGeo>], head_on: &[bool],
                head_on_dist: f32, ovr: &mut [crate::ai::Overrides]) -> bool {
    let (near, a_win, b_win) = if arrow { (10.0f32, (135.0, 160.0), (20.0, 45.0)) } else { (15.0f32, (160.0, 180.0), (0.0, 20.0)) };
    let geo = |k: Option<usize>| k.and_then(|k| g.get(k).copied().flatten().map(|x| (k, x)));
    let mut line = false;
    match geo(a) {
        Some((k, x)) if x.dsq < near * near && x.same_dir && !(arrow && x.ahead) && !x.on_right && x.bearing > a_win.0 && x.bearing < a_win.1 => {
            if !sq.latch {
                line = true;
                if arrow {
                    sq.swipe_t = 0.5;
                }
            }
            if sq.swipe_t == 0.0 {
                ovr[k].steer = Some(-1.0);
                ovr[k].brake = Some(1.0);
            }
            sq.latch = true;
        }
        _ => {
            sq.swipe_t = 0.0;
            sq.latch = false;
        }
    }
    if let Some((k, x)) = geo(b) {
        if x.dsq < near * near && x.same_dir && !(arrow && x.ahead) && x.on_right && x.bearing > b_win.0 && x.bearing < b_win.1 {
            ovr[k].steer = Some(1.0);
            ovr[k].brake = Some(1.0);
        }
    }
    if sq.swipe_t > 0.0 {
        sq.swipe_t -= dt;
        if sq.swipe_t < 0.0 {
            sq.swipe_t = 0.0;
        }
    }
    for (k, x) in g.iter().enumerate() {
        if let (Some(x), true) = (x, head_on.get(k).copied().unwrap_or(false)) {
            if x.dsq < head_on_dist * head_on_dist {
                ovr[k].handbrake = Some(true);
                ovr[k].steer = Some(if x.on_right { 1.0 } else { -1.0 });
            }
        }
    }
    line
}

/// fmadds: a x b + c in double, rounded once to single
fn fmas(a: f32, b: f32, c: f32) -> f32 {
    (a as f64).mul_add(b as f64, c as f64) as f32
}

/// The squad formation, one call (the Arrow Trap's F-80532, the Double Pursuit's F-21534), on cop indices. The slots: the
/// right flank's (`slots[0]`) route point (F-32694, -1000 all round for a brain with none) when it is on the target's right
/// (F-41266), else `along` to the right (`right`: the target matrix's first row); the left flank's when it is not, else
/// `along` to the left; the Arrow Trap's third `along` along the matrix's third row (`back`). The cops go to the slots of the
/// smallest sum of squared distances (slot to cop, in the original's order; the first of equal sums). A cop whose brain
/// (`brains[k]`: its state, the side byte +708) is not the one wanted - AlongSide 5 on side 1 / 0, the Arrow Trap's third
/// Pursue 4 - is sent it (F-24671, which sets it at once) when the hysteresis was over 1 s before this call; the hysteresis
/// grows by dt while one is wanting, else 0, and 0 after a send. After a send the slots: the Arrow Trap's from the third
/// cop and the flank after it in turn (F-13075), the Double Pursuit's from the brains when both are AlongSide, else none
/// (as at its AlongSide's enter, F-80770: `enter`). Returns whether it sent.
#[allow(clippy::too_many_arguments)]
fn squad_formation(hyst: &mut f32, slots: &mut [Option<usize>; 3], arrow: bool, brains: &mut [(u8, bool)], pos: &[Vec3],
                   route: &[Option<Vec3>], tpos: Vec3, right: Vec3, back: Vec3, along: f32, dt: f32, enter: bool) -> bool {
    let both_along = |b: &[(u8, bool)]| (b[0].0 == 5 && b[1].0 == 5).then(|| if b[0].1 { [Some(0), Some(1), None] } else { [Some(1), Some(0), None] });
    if enter && !arrow {
        *slots = both_along(brains).unwrap_or([None; 3]);
    }
    let point = |k: Option<usize>| k.map(|k| route[k].unwrap_or(Vec3::splat(-1000.0)));
    let right_of = |q: Vec3| {
        let d = q - tpos;
        fmas(right.x, d.x, fmas(right.z, d.z, right.y * d.y)) > 0.0
    };
    let s = [
        point(slots[0]).filter(|&q| right_of(q)).unwrap_or(tpos + right * along),
        point(slots[1]).filter(|&q| !right_of(q)).unwrap_or(tpos - right * along),
        tpos + back * along,
    ];
    let perms: &[[usize; 3]] = if arrow { &[[0, 1, 2], [0, 2, 1], [1, 0, 2], [1, 2, 0], [2, 0, 1], [2, 1, 0]] } else { &[[0, 1, 2], [1, 0, 2]] };
    let n = if arrow { 3 } else { 2 };
    let sum = |p: &[usize; 3]| (0..n).fold(0.0f32, |acc, j| {
        let d = s[j] - pos[p[j]];
        acc + fmas(d.x, d.x, fmas(d.z, d.z, d.y * d.y))
    });
    let mut best = (sum(&perms[0]), perms[0]);
    for p in &perms[1..] {
        let v = sum(p);
        if v < best.0 {
            best = (v, *p);
        }
    }
    let (mut want, mut sent) = (false, false);
    for j in 0..n {
        let k = best.1[j];
        let wanted = if j < 2 { (5, j == 0) } else { (4, brains[k].1) };
        if brains[k] != wanted {
            want = true;
            if *hyst > 1.0 {
                brains[k] = wanted;
                sent = true;
            }
        }
    }
    *hyst = if want { *hyst + dt } else { 0.0 };
    if sent {
        *slots = if arrow {
            let t = best.1[2];
            let (a, b) = ((t + 1) % 3, (t + 2) % 3);
            if brains[a].1 { [Some(a), Some(b), Some(t)] } else { [Some(b), Some(a), Some(t)] }
        } else {
            both_along(brains).unwrap_or([None; 3])
        };
        *hyst = 0.0;
    }
    sent
}

/// Dispatch to Location's transition test (F-40040): WaitToDispatch only to Dispatch, Dispatch only to Respond, Respond only
/// to WaitAtLocation, nothing out of WaitAtLocation (its end is the tactic's result). The machine starts in WaitToDispatch: the
/// tactic's begin starts it with its enter flag set (F-26187).
fn dispatch_may_go(from: u8, to: i32) -> bool {
    match from {
        0 => to == 1,
        1 => to == 2,
        2 => to == 3,
        _ => false,
    }
}

/// Dispatch to Location's machine, the start of its update (F-95322): the state asked for in the last update is entered
/// (F-43677; its time 0, Respond and WaitAtLocation start unrouted, WaitAtLocation follows the trail when it has crumbs).
/// Returns the state entered.
fn dispatch_enter(d: &mut Dispatch) -> Option<u8> {
    let n = d.next.take()?;
    d.st = n;
    d.t = 0.0;
    if n >= 2 {
        d.routed = false;
        d.arrived = false;
        d.go_t = 0.0;
    }
    if n == 3 {
        d.trail = !d.crumbs.is_empty();
    }
    Some(n)
}

/// The states' tests on their time before this update's dt (the state ran first, F-88920; a state asked for is stored,
/// F-28114): WaitToDispatch (F-64066) after `delay` (fDispatchRdoDelayTime) with no career speech line playing (nor the
/// form-table entry 109's +3636 set - not identified, not ported); Dispatch (F-52633) at once unless a career speech line
/// plays (the police radio has a manager of its own); Respond once arrived.
fn dispatch_ask(d: &mut Dispatch, delay: f32, speech_busy: bool) {
    let st = d.st;
    match st {
        0 if d.t > delay && !speech_busy && dispatch_may_go(st, 1) => d.next = Some(1),
        1 if !speech_busy && dispatch_may_go(st, 2) => d.next = Some(2),
        2 if d.arrived && dispatch_may_go(st, 3) => d.next = Some(3),
        _ => {}
    }
}

/// A cop needs a teleport (F-14059): underground (+880), or its brain over 5 s off the nav mesh (+584 > 5), or its car damaged
/// out with the DamageOutDelay countdown (+44) not over 0.
fn needs_teleport(under: bool, off_mesh_t: f32, out: bool, out_delay: f32) -> bool {
    under || off_mesh_t > 5.0 || (out && !(out_delay > 0.0))
}

/// A start report as the radio says it: `default` = the default report class (F-56787: REPORT_VIOLATION_PURSUIT,
/// probability 0.2), whose send F-66952 first draws the shared generator and makes the probability 1.0 under 0.25; then
/// F-14569 (a name set) draws again and says the name under the probability; else by +8 (`bits`, the law record's bits):
/// 0x20 REPORT_HIT_COP_CAR_HARD, 0x40 _SOFT, 0x80 REPORT_VIOLATION_MAJORINCIDENT, 0x200 _REDLIGHTS, 0x8
/// _RECKLESS_SIDEWALK, 0x2000 _RECKLESS_WRONGSIDE, 0x7 _SPEEDING, none: the name. The other report classes have
/// probability 1 (one draw, the name). `draw` is the generator's 23-bit float.
pub fn report_line(name: &'static str, default: bool, bits: u32, mut draw: impl FnMut() -> f32) -> &'static str {
    let mut prob = if default { 0.2f32 } else { 1.0 };
    if default && draw() < 0.25 {
        prob = 1.0;
    }
    report_say(Some(name).filter(|n| !n.is_empty()), prob, bits, draw).unwrap_or(name)
}

/// F-14569 on its own: a name (`None`: no name or an empty one) draws once and is said under `prob`; else the first bit
/// group of `bits` picks the line; none - the name, or nothing (`None`) without a name.
fn report_say(name: Option<&'static str>, prob: f32, bits: u32, mut draw: impl FnMut() -> f32) -> Option<&'static str> {
    if let Some(n) = name {
        if draw() < prob {
            return Some(n);
        }
    }
    const BY_BIT: [(u32, &str); 7] = [
        (0x20, "REPORT_HIT_COP_CAR_HARD"),
        (0x40, "REPORT_HIT_COP_CAR_SOFT"),
        (0x80, "REPORT_VIOLATION_MAJORINCIDENT"),
        (0x200, "REPORT_VIOLATION_REDLIGHTS"),
        (0x8, "REPORT_VIOLATION_RECKLESS_SIDEWALK"),
        (0x2000, "REPORT_VIOLATION_RECKLESS_WRONGSIDE"),
        (0x7, "REPORT_VIOLATION_SPEEDING"),
    ];
    BY_BIT.iter().find(|b| bits & b.0 != 0).map(|b| b.1).or(name)
}

/// What the tactic end F-67316 does to one cop of its array (a null entry is skipped): a car with a brain gets the brain
/// command 0 (F-24671), its lost flag +773 = 1 and +408 = 0; with the flag's low byte set the siren stop F-75318, and when
/// the result +24 is not 1 (arrested) the siren target 0 (F-78189) and, with no race current (F-27863), the stream unlock
/// F-93264 - in that order. `None` when the tactic is not active (+20 clear: the end returns 0 and does nothing).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct CopEnd {
    brain_reset: bool,
    siren_stop: bool,
    lights_off: bool,
    unlock: bool,
}

fn tactic_end(active: u8, flag: u32, result: u32, racing: bool, brain: bool) -> Option<CopEnd> {
    if active == 0 {
        return None;
    }
    let flag = flag & 0xFF != 0;
    let lights_off = flag && result != 1;
    Some(CopEnd { brain_reset: brain, siren_stop: flag, lights_off, unlock: lights_off && !racing })
}

/// F-71212's states 2 and 3 (tools/zone_setoff_reference.py, the ticks): 2 - the target damaged out (F-12032, asked only
/// with a target) starts the chase at once (state 0), else the timer +212 down by dt, below 0 state 3 with
/// fCopZoneRdoDelayTime; 3 - the timer down, and with a target, below 0 or (only then asked) damaged out, the chase (state
/// 0, the timer kept): the Arrow Trap (StartTactic F-33550 type 4, F-30913's tactic on the target). (state, timer, start)
pub(crate) fn zone_tick(state: u8, target: bool, timer: f32, dt: f32, delay: f32, mut wrecked: impl FnMut() -> bool) -> (u8, f32, bool) {
    match state {
        2 if target && wrecked() => (0, timer, true),
        2 => {
            let t = timer - dt;
            if t < 0.0 { (3, delay, false) } else { (2, t, false) }
        }
        3 => {
            let t = timer - dt;
            if target && (t < 0.0 || wrecked()) { (0, t, true) } else { (3, t, false) }
        }
        _ => (state, timer, false),
    }
}

/// The zone's target (manager +200): a racer of the race (its index) or an entry of the AI list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ZoneTgt {
    Racer(usize),
    Ai(usize),
}

/// The zone machine's squared distance (F-71212): d = a - b, dy dy, then dz and dx fused in.
fn zone_d2(a: Vec3, b: Vec3) -> f32 {
    let (dx, dy, dz) = (a.x - b.x, a.y - b.y, a.z - b.z);
    fmas(dx, dx, fmas(dz, dz, dy * dy))
}

/// The cop zone's set-off, F-71212's state 1 (tools/zone_setoff_reference.py): the race's racers in order; the first within
/// the zone's radius (`zone_d2` < r x r) sets it off (a request each racer inside), and its target: the scripted racer
/// (SetRaceChaseTarget's +208, when it is in the racer list), else by the kind +204 - 1 the racer; 0 and 2 with no AI list
/// the racer; 0 one draw of the shared generator, (lo & 0x7FFFFF) x 2^-23 < iRaceTgtAiProbPrcnt x 0.01 or the racer; then
/// (0 and 2) the nearest entry of the AI list (count 0 or entry 0 null: the racer; null entries skipped; the first of the
/// equal) when fRaceTgtAiDist is negative or it is nearer than that, else the racer; 3 and up none, and the next racer
/// inside sets it off again. None: nothing set it off; else (the requests, the target).
pub(crate) fn zone_set_off(rng: &mut (u32, u32), kind: u32, prob: i32, dist: f32, zone: Vec3, r: f32, racers: &[Vec3], scripted: Option<usize>,
                           ai: Option<&[Option<Vec3>]>) -> Option<(u32, Option<ZoneTgt>)> {
    let mut req = 0;
    for (k, &rp) in racers.iter().enumerate() {
        if !(zone_d2(rp, zone) < r * r) {
            continue;
        }
        req += 1;
        if let Some(s) = scripted {
            return Some((req, Some(ZoneTgt::Racer(s))));
        }
        let tgt = match kind {
            1 => ZoneTgt::Racer(k),
            0 | 2 => match ai {
                None => ZoneTgt::Racer(k),
                Some(list) => {
                    let rolled = kind == 2 || {
                        let d = (rdo_draw(rng) & 0x7F_FFFF) as f32 * f32::from_bits(0x3400_0000);
                        d < prob as f32 * 0.01f32
                    };
                    match list.first() {
                        Some(&Some(q0)) if rolled => {
                            let (mut best, mut bd) = (0, zone_d2(rp, q0));
                            for (j, q) in list.iter().enumerate().skip(1) {
                                if let Some(q) = *q {
                                    let d = zone_d2(rp, q);
                                    if d < bd {
                                        (best, bd) = (j, d);
                                    }
                                }
                            }
                            if dist < 0.0 || bd < dist * dist { ZoneTgt::Ai(best) } else { ZoneTgt::Racer(k) }
                        }
                        _ => ZoneTgt::Racer(k),
                    }
                }
            },
            _ => continue,
        };
        return Some((req, Some(tgt)));
    }
    if req > 0 { Some((req, None)) } else { None }
}

/// EvaluateCopZones' pick F-88125 (after it cleared the zone: +188 / +192 -1, +196 0): "no cops" (`None`) when the dev
/// override F-47443 is set with a value <= 0 (`ovr`), the manager's police list is empty (F-53190) or the race has no zones
/// (+184); else the zone is one index draw of the shared generator ((lo & 0x7FFFFFFF) % zones) and the lap 0, or with a race
/// current (`laps`: its +3124, `None` without a race; the code's test of race + 8 is on the address, never null) of more than one lap a second draw % laps. The
/// caller arms lap 0 (+196 = 1); the original then calls SetAllPoliceUnderground(1, 1) F-36452 (the port's race start has
/// its units underground already).
fn zone_pick(rng: &mut (u32, u32), ovr: Option<i32>, police: i32, zones: u16, laps: Option<i32>) -> Option<(i32, i32)> {
    if ovr.map_or(false, |v| v <= 0) || police <= 0 || zones == 0 {
        return None;
    }
    let idx = (rdo_draw(rng) % zones as u32) as i32;
    let lap = match laps {
        Some(l) if l > 1 => (rdo_draw(rng) % l as u32) as i32,
        _ => 0,
    };
    Some((idx, lap))
}

/// The track timer's step in the common update F-63286: +136 runs down by dt while above 0 (no clamp); the step that takes it
/// to <= 0 sets the brains' lost flag +773 when the tactic's vt+28 answers (`answers`: the Pursuit, Double Pursuit and Arrow
/// Trap classes give their +29, the first chase property; the pull-over 0).
fn track_step(track_t: &mut f32, lost_ok: &mut bool, dt: f32, answers: bool) {
    if *track_t > 0.0 {
        *track_t -= dt;
        if *track_t <= 0.0 && answers {
            *lost_ok = true;
        }
    }
}

/// Damaged out (F-12032, mcCarDamage +36 >= its tune's MaxDamage +24, as `!(points < max)`: a NaN is out). The port keeps
/// damage as points / MaxDamage, capped at 1.0, and points == MaxDamage give exactly 1.0; a car one point - or a fraction of
/// one - short is not out (the port had tested 0.999, which took points from 0.999 x MaxDamage).
pub fn damaged_out(damage: f32) -> bool {
    !(damage < 1.0)
}

/// StateTicket's tick F-46174 and the state's clock: true (the tactic's result, +24 = 1) once the state's time - before
/// this update's dt is added - is over the limit (fArrestStateTime, + 0.5 s when the enter F-25878 left +68 at 0: the
/// target damaged out); the time then grows by dt. `hold` = (time, limit).
fn ticket_tick(hold: &mut (f32, f32), dt: f32) -> bool {
    let over = hold.0 > hold.1;
    hold.0 += dt;
    over
}

/// "Moving" (F-43975 for the target, F-36212 for the cop's own car): the speed over fMovingSpeed x 0.447.
fn moving(speed: f32, moving_speed: f32) -> bool {
    speed > moving_speed * 0.447
}

/// The default final speed of a brain state (F-85634): the target car's speed (F-41260) when it is moving (F-43975), else 0.0;
/// no target car - 0.0.
fn default_final_speed(target_speed: Option<f32>, moving_speed: f32) -> f32 {
    match target_speed {
        Some(v) if moving(v, moving_speed) => v,
        _ => 0.0,
    }
}

/// A junction's arm vector while green (F-75755: the timer +164 < 6.5) / while yellow (F-93041: not < 6.5 and < 8.5): the arm
/// at the junction's current index +160 when 0 <= it < the arm count, else the zero vector. A NaN timer is neither.
fn arm_green(timer: f32, current: i32, arms: &[Vec3]) -> Vec3 {
    let ok = timer < 6.5 && current >= 0 && (current as usize) < arms.len();
    if ok { arms[current as usize] } else { Vec3::ZERO }
}

fn arm_yellow(timer: f32, current: i32, arms: &[Vec3]) -> Vec3 {
    let ok = !(timer < 6.5) && timer < 8.5 && current >= 0 && (current as usize) < arms.len();
    if ok { arms[current as usize] } else { Vec3::ZERO }
}

/// The running arm as the law checks take it (the red light, block traffic): the green arm, and when that is the zero vector
/// the yellow one.
fn running_arm(timer: f32, current: i32, arms: &[Vec3]) -> Vec3 {
    let g = arm_green(timer, current, arms);
    if g != Vec3::ZERO { g } else { arm_yellow(timer, current, arms) }
}

/// The pull-over's may-escalate (its vt+24, F-98647): false while the bullhorn timer +112 > 0 (fcmpu: NaN passes), else the base
/// test F-38605 on the pull-over's slots - its Search test vt+40 is F-74264 (always false), its vt+36 F-33778 the FSM in
/// state 3 (Ticket). AddPoints (F-19359) lifts a pull-over's 599 cap only when the bullhorn has sounded (+92 == 2) and this is true.
fn pullover_may_escalate(may: bool, arrest: f32, bullhorn: f32, ticket: bool) -> bool {
    !(f64::from(bullhorn) > 0.0) && tactic_may_escalate(false, may, false, ticket, arrest)
}

/// A Pursuit's may-escalate (its vt+24): not in its Delay (state 0), then the radio's busy byte and the base test F-38605 on its
/// slots (vt+40 state 6 Search, vt+36 state 4 Arrest).
fn pursuit_may_escalate(in_delay: bool, radio_busy: bool, may: bool, searching: bool, arresting: bool, arrest: f32) -> bool {
    !in_delay && tactic_may_escalate(radio_busy, may, searching, arresting, arrest)
}

/// The law tactic on the target as AddPoints reads it: its type (vt+56), the may-escalate byte +28, the pull-over's bullhorn
/// sounded (+92 == 2) and its vt+24.
#[derive(Clone, Copy, Debug)]
struct LawTactic {
    ty: i32,
    may: bool,
    sounded: bool,
    escalates: bool,
}

/// AddPoints (F-19359): with a tactic on the target whose may-escalate byte is 0 the points are dropped (None). The cap is
/// iMaxLawBreakingScore; a Pull Over's iPursuitLawBreakScore - 1 unless its bullhorn has sounded and its vt+24 holds; a Pursuit's /
/// Double Pursuit's iMax - 1 unless its vt+24 holds. The sum is clamped to 0 .. cap (a score above the cap comes down to it).
fn add_points(score: i32, points: i32, tac: Option<LawTactic>, max: i32, pursuit: i32) -> Option<i32> {
    let mut cap = max;
    if let Some(t) = tac {
        if !t.may {
            return None;
        }
        match t.ty {
            1 if !(t.sounded && t.escalates) => cap = pursuit - 1,
            2 | 3 if !t.escalates => cap = max - 1,
            _ => {}
        }
    }
    let v = score.wrapping_add(points);
    Some(if v < 0 { 0 } else { v.min(cap) })
}

/// The brain's GoToPosition tick (F-16378, its state 8): the mode word +576 - what Dispatch's Respond reads as the arrival
/// (F-98680 == 1) - is latched to 1 when the brain has been within 8 m of its goal for over 5 s (+580), or when the state's time
/// is above 0 and the route is done (HasReachedGoal F-43234); else it is left. The time is counted after the tick (from 0 at the
/// command), so the route is not tested on the state's first tick.
fn go_to_arrived(arrived: bool, near_t: f32, state_t: f32, route_done: bool) -> bool {
    arrived || near_t > 5.0 || (state_t > 0.0 && route_done)
}

/// A cop's brain Search starting: the state's reset F-97098 (no points, the route not set - its last index +344 is written only),
/// then its enter's point list (F-57507 -> F-52110).
fn search_start(uid: u32, pts: Vec<(Vec3, Option<NavNode>)>, random: bool, angle: f32) -> CopSearch {
    CopSearch { uid, no_goals: pts.is_empty(), pts, random, angle, ..Default::default() }
}

/// SetRaceChaseTarget (F-12315): the zone's chase type +204 - a racer given: 1 when it is the player (its vt+40), else 2;
/// none: the type given.
const fn race_chase_type(ty: u8, racer_is_player: Option<bool>) -> u8 {
    match racer_is_player {
        None => ty,
        Some(true) => 1,
        Some(false) => 2,
    }
}

/// The cop unit's patrol setter (F-30531, (group, path)): a group >= 0 sets the hold +1128 to fPatrolTime (tune +316); a
/// detach (-1) leaves it.
fn patrol_hold(group: i32, hold: f32, patrol_time: f32) -> f32 {
    if group >= 0 { patrol_time } else { hold }
}

/// F-43223: two frames face the same way - their forward rows dotted y, z, x (the y product rounded, the sums fused)
/// over 0 (the port's forward is the row negated, which leaves the product's bits).
fn same_facing(a: Vec3, b: Vec3) -> bool {
    crate::physics::exact::dot_xzy(a, b) > 0.0
}

/// F-57435: the point `p` on the +x side of the frame at `pos` - its row 0 dotted with p - pos as F-43223 dots.
fn on_side(row0: Vec3, pos: Vec3, p: Vec3) -> bool {
    crate::physics::exact::dot_xzy(row0, p - pos) > 0.0
}

/// The original's 2-D unit vector (F-98766): `or` when x x + y y (the y product rounded, the sum fused) is under 1e-12,
/// else both times the reciprocal square root's estimate and two Newton steps in double, rounded once to single.
fn unit2(v: glam::Vec2, or: glam::Vec2) -> glam::Vec2 {
    let l = crate::physics::exact::fma(v.x, v.x, v.y * v.y);
    if l < 1e-12 {
        return or;
    }
    let l = l as f64;
    let mut r = (1.0 / l.sqrt()) as f32 as f64;
    for _ in 0..2 {
        r = (-(r * r).mul_add(l, -1.0)).mul_add(r * 0.5, r);
    }
    let r = r as f32;
    glam::Vec2::new(v.x * r, v.y * r)
}

/// The inputs of the cop brain's goal plan (F-67303's replan with the per-state destinations: Pursuit F-15067 / F-49680,
/// PullOver F-51787, AlongSide F-13443, the junction prediction and the quick re-aim).
#[derive(Clone, Copy, Debug)]
struct PlanIn {
    bcmd: u8,
    ed78: bool,
    head_on: bool,
    quick: bool,
    turned: bool,
    t_moving: bool,
    same_dir: bool,
    ahead: bool,
    /// the law record's frame offence bit 0x2000 (wrong way, F-88545's gated bit)
    wrong_way: bool,
    /// the target's lane tangent (its +624) when it has a lane (+640)
    lane_tan: Option<Vec3>,
    side: u8,
    ppos: Vec3,
    pvel: Vec3,
    pfwd: Vec3,
    pright: Vec3,
    cpos: Vec3,
    /// brain +640: the target's position last seen (the quick re-aim's goal)
    last_seen: Vec3,
    speed: f32,
    d: f32,
    safe: f32,
    along: f32,
    /// fMinSetGoalTime (tune +208) as the brains read it: not halved head-on (only the countdown is), the head-on
    /// chains double it instead
    per: f32,
    moving_speed: f32,
    pull_dist: f32,
}

/// The brain's nav test as F-72689 / F-45363 make it: over the navigation mesh, or a polygon within 10 m - then the point
/// becomes that polygon's stored centre unless it is inside it (true with no mesh loaded).
fn nav_snap(q: &mut Vec3) -> bool {
    let Some(m) = crate::offroad::mesh() else { return true };
    if m.on_mesh(*q) {
        return true;
    }
    match m.teleport_target(*q) {
        Some(c) => {
            *q = c;
            true
        }
        None => false,
    }
}

/// The cop brain's goal plan (see `PlanIn`): the two goals and the speed to reach the last with.
fn cop_plan(nav: Option<&NavSys>, i: &PlanIn) -> ([Vec3; 2], f32) {
    let PlanIn { bcmd, ed78, head_on, quick, turned, t_moving, same_dir, ahead, wrong_way, lane_tan, side, ppos, pvel, pfwd, pright, cpos, last_seen, speed, d, safe, along, per, moving_speed, pull_dist } = *i;
    let on_road = |q: Vec3| crate::offroad::mesh().map_or(true, |m| m.on_road(q));
    let ho = if head_on { 2.0 } else { 1.0 };
    
    // (F-72689 over the mesh, or F-45363 a polygon within 10 m - which writes that polygon's stored centre into the point
    // when it is not inside it: `nav_snap`)
    let nav_ok = |q: &mut Vec3| nav_snap(q);
    // F-19210: the lane within 5 m of q (F-59579: its derivative there, not unit) gives the direction - its velocity
    // itself when there is none, it is a junction's (F-41621) or zero; turned to the velocity's side (the fused dot), and
    // when it is under 45 deg (ang3, F-88594) off the velocity, made unit as the vector unit does (vmsum3fp, 1 / sqrt
    // with infinity to 0, one Newton step) and scaled to the target's speed; else the velocity
    let dir_at = |q: Vec3| -> Vec3 {
        use crate::physics::exact::{dot3, fma};
        let lane = nav.and_then(|n| crate::vehicles::traffic::snap_to_lane(n, q, 5.0).0.filter(|h| n.lane_junction.get(h.lane).map_or(true, |j| j.is_none())));
        let Some(mut l) = lane.map(|h| h.dir).filter(|l| *l != Vec3::ZERO) else { return pvel };
        if fma(l.x, pvel.x, fma(l.z, pvel.z, l.y * pvel.y)) < 0.0 {
            l = Vec3::ZERO - l;
        }
        if crate::aiv::ang3(l, pvel) >= std::f32::consts::FRAC_PI_4 {
            return pvel;
        }
        let d = dot3(l, l);
        let e = 1.0 / d.sqrt();
        let e = if e == f32::INFINITY { 0.0 } else { e };
        l * ((1.0 - d * (e * e)) * (0.5 * e) + e) * speed
    };
    // the two-goal chain from b, k seconds of D each; a goal off the mesh is moved back and forth
    // along D by a quarter of k (4 tries each way, from the goal; D read again at the last point tried)
    let chain = |b: Vec3, k: f32| -> [Vec3; 2] {
        let mut out = [b; 2];
        let mut q = b;
        // (the goal is the point before the test; the next goal chains from the point the test left - a polygon's centre)
        for g in out.iter_mut() {
            let p0 = q + dir_at(q) * k;
            *g = p0;
            q = p0;
            if !nav_ok(&mut q) {
                for j in 1..=4 {
                    let dd = dir_at(q);
                    let off = j as f32 * 0.25;
                    q = p0 + dd * (k - off);
                    if nav_ok(&mut q) {
                        *g = q;
                        break;
                    }
                    q = p0 + dd * (off + k);
                    if nav_ok(&mut q) {
                        *g = q;
                        break;
                    }
                }
            }
        }
        out
    };
    if quick && turned {
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
        // (F-90092 over a road, F-53622 a road polygon within 10 m, F-72689, F-45363 - the two within-10 m ones move the
        // point onto their polygon's centre)
        let q = pick.and_then(|mut q| {
            let m = crate::offroad::mesh();
            if on_road(q) {
                return Some(q);
            }
            if let Some(c) = m.and_then(|m| m.road_teleport_target(q)) {
                return Some(c);
            }
            nav_ok(&mut q).then_some(q)
        }).unwrap_or(last_seen);
        ([q, q], speed)
    } else if quick {
        ([last_seen, last_seen], speed)
    } else {
        // the target's own speed when it is moving, else 0 (F-85634)
        let default_v = default_final_speed(Some(speed), moving_speed);
        match bcmd {
            // Pursuit brain (F-15067): from the target, P and 2P (head-on 2P, 4P) along D; a stopped target:
            // from fSafeDist off it towards the cop. Final speed (F-49680): 0 for a stopped target, full
            // pace (-1) beyond 2 x fSafeDist, 1.2 x its speed when it is near, going the same way and
            // ahead of the cop, else full pace
            // (F-42951 false - a police cop in a race: from the target less P of its velocity, 1 s steps,
            // the default final speed)
            4 if !ed78 => {
                let b = if t_moving { ppos - pvel * per } else { ppos + (cpos - ppos).normalize_or(Vec3::X) * safe };
                (chain(b, 1.0), default_v)
            }
            4 => {
                let b = if t_moving { ppos } else { ppos + (cpos - ppos).normalize_or(Vec3::X) * safe };
                let v = if !t_moving { 0.0 } else if d >= 2.0 * safe { -1.0 } else if same_dir && ahead { 1.2 * speed } else { -1.0 };
                (chain(b, per * ho), v)
            }
            // PullOver brain (F-51787): one point, both goals - the target; 15 m behind it stopped
            // (fPullOverDist); driving the wrong way, P of its velocity on and 8 m to the side of its lane;
            // off the mesh, back by P of its velocity up to 5 times
            // (the law record's wrong-way bit 0x2000 and the target's lane +640 / its tangent +624; vector
            // multiply-adds as the original's vmaddfp: product rounded, then the add)
            3 => {
                let q = match (wrong_way, lane_tan) {
                    (true, Some(tn)) => (ppos + pvel * per) + crate::aiv::rot_y(tn, std::f32::consts::FRAC_PI_2) * 8.0,
                    (true, None) => ppos,
                    _ if !t_moving => ppos + (-pfwd) * pull_dist,
                    _ => ppos,
                };
                let mut g = q;
                if !nav_ok(&mut g) {
                    let step = pvel * per;
                    let mut b = q;
                    for _ in 0..5 {
                        b -= step;
                        if nav_ok(&mut b) {
                            g = b;
                            break;
                        }
                    }
                }
                ([g, g], default_v)
            }
            // AlongSide brain (F-13443): from fAlongSideDist beside the target, 1.15 P (head-on 2.3 P) along D
            _ if !ed78 => {
                let b = ppos + pright * if side != 0 { along } else { -along } - pvel * per;
                (chain(b, 1.0), default_v)
            }
            _ => {
                let b = ppos + pright * if side != 0 { along } else { -along };
                (chain(b, ho * per * 1.15), default_v)
            }
        }
    }
}

/// The search's points (F-52110): from `start` along the target's last known velocity `v` (at least 22.35 m/s)
/// x fMinSetGoalTime `per` until a point lies on a road or junction (within 3 m), then along the connection list
/// to the neighbour best in line with that direction, each next point the middle of the node's lane; 10 at most.
fn search_points(nav: &NavSys, world: &World, start: Vec3, v: Vec3, per: f32, first: bool, random: bool, angle: f32, rnd: &mut dyn FnMut() -> f32) -> Vec<(Vec3, Option<NavNode>)> {
    let probe = |p: Vec3, h: f32| -> Option<Vec3> { world.collider.raycast(p + Vec3::Y * h, -Vec3::Y, 2.0 * h).map(|hit| hit.point) };
    search_points_on(nav, crate::offroad::mesh(), &probe, start, v, per, first, random, angle, rnd)
}

/// `search_points` on a given navigation mesh and vertical probe (`probe(p, h)`: the line from p + h up to p - h down,
/// its hit point; F-20855).
#[allow(clippy::too_many_arguments)]
fn search_points_on(nav: &NavSys, mesh: Option<&mcla_formats::rage::navmesh::NavMesh>, probe: &dyn Fn(Vec3, f32) -> Option<Vec3>, start: Vec3, v: Vec3, per: f32, first: bool, random: bool, angle: f32, rnd: &mut dyn FnMut() -> f32) -> Vec<(Vec3, Option<NavNode>)> {
    // F-58319: over the mesh, or F-45363's polygon within 10 m - which moves the point to its centre
    let nav_ok = |p: Vec3| -> Option<Vec3> {
        match mesh {
            None => Some(p),
            Some(m) if m.on_mesh(p) => Some(p),
            Some(m) => m.teleport_target(p),
        }
    };
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
        let mut dir = unit2(glam::Vec2::new(s.x, s.z), glam::Vec2::ZERO);
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
                    // (the guest runtime's sine and cosine of the turn, the products of x rounded, the sums fused)
                    let (sn, cs) = (crate::physics::exact::guest_sin(angle as f64) as f32, crate::physics::exact::guest_cos(angle as f64) as f32);
                    let t = glam::Vec2::new(crate::physics::exact::fma(dir.y, sn, dir.x * cs), crate::physics::exact::fms(dir.y, cs, dir.x * sn));
                    dir = unit2(t, glam::Vec2::ZERO);
                }
                turn = random && rnd() < 0.5;
            }
            // (F-59248: the node's lane at its middle; F-90092 on a road, else F-53622 - the road polygon within 10 m,
            // which moves it there -, else nav_ok)
            let Some(lane) = nav.node_rect(n).lane else { break };
            let Some(q) = nav.curves.get(lane).map(|c| crate::railroute::point(c, 0.5)) else { break };
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

/// The squared distance as the original's code forms it (F-39842 and the +144 test of F-63286): the y step squared, then
/// the z and the x steps fused in (fmadds: the product and sum in double, rounded to single, as the recompiled code).
pub(crate) fn arrest_d2(a: Vec3, b: Vec3) -> f32 {
    let (dy, dz, dx) = (b.y - a.y, b.z - a.z, b.x - a.x);
    let t = (dz as f64).mul_add(dz as f64, (dy * dy) as f64) as f32;
    (dx as f64).mul_add(dx as f64, t as f64) as f32
}

/// "Within r" of F-39842 (F-29443's cop test): the squared distance strictly under r x r.
pub(crate) fn within_sq(a: Vec3, b: Vec3, r: f32) -> bool {
    arrest_d2(a, b) < r * r
}

/// The speed under which the arrest counts the target slow (F-29906) and the step under which it counts it barely moving
/// (F-63286 x dt): fArrestTgtThresholdSpd x 0.447, a literal of the code (not 1 / 2.23694).
pub(crate) fn arrest_ms(tune: f32) -> f32 {
    tune * 0.447
}

/// A line of the tactic's radio: its scenario and the cars of its parties (no receiver: the dispatcher).
pub struct RadioCall {
    pub name: &'static str,
    pub sender: u32,
    pub receiver: Option<u32>,
}

/// The tactic radio's own state (F-26639): the countdown +112, the law record's road +60 and its timer +64, the nitrous
/// tries +100, the far byte +96, and the radio's draw state (the game's one generator, shared with the lane changes).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct RdoState {
    pub t: f32,
    pub road: Option<usize>,
    pub fw_t: f32,
    pub nitro: u8,
    pub far: bool,
    pub rng: (u32, u32),
}

/// What `chatter_step` reads from the world (the callees of F-26639 that are not its own code).
pub(crate) struct RdoIn<'a> {
    pub dt: f32,
    /// the tactic kind's vt+80 (Pursuit past its Delay, Double Pursuit, Arrow Trap; not in the Search)
    pub on: bool,
    /// F-94735: a speech line is playing
    pub busy: bool,
    /// the tactic's cops (+72)
    pub n: usize,
    /// the target (+32) exists
    pub target: bool,
    /// the target's vt+40 (a player)
    pub is_player: bool,
    /// the law record (+36) exists
    pub rec: bool,
    /// F-86691: the road of the target's lane (none: no lane or no road)
    pub road: Option<usize>,
    /// F-28783: the road's name starts FW or XT
    pub is_fw: &'a dyn Fn(usize) -> bool,
    /// the law record's hard-hit bit (+0 & 0x20)
    pub hard: bool,
    /// F-13798 per cop: the value of the cop's entry in the target's contact list (none: no +48 or no entry)
    pub hit: &'a [Option<f32>],
    /// the target's nitrous burning byte
    pub nitro_burning: bool,
    /// the target's speed (+48 +8 +224); 0 when the chain is broken or there is no target (the candidate is skipped then)
    pub speed: f32,
    /// F-27863: a race is current
    pub in_race: bool,
    /// F-76898: the manager's +80
    pub mgr80: i32,
    /// F-29443 at fRdoVicinityDist / at 10 m
    pub within_vic: bool,
    pub within_10: bool,
    pub min_t: f32,
    pub max_t: f32,
}

/// The radio's draw (F-26639): x = lo x 0x5CDCFAA7 + hi; low and high 32 bits the new state.
pub(crate) fn rdo_draw(rng: &mut (u32, u32)) -> u32 {
    let x = rng.0 as u64 * 0x5CDC_FAA7 + rng.1 as u64;
    *rng = (x as u32, (x >> 32) as u32);
    x as u32 & 0x7FFF_FFFF
}

fn rdo_cop(rng: &mut (u32, u32), n: usize) -> usize {
    if n > 1 { rdo_draw(rng) as usize % n } else { 0 }
}

/// One update of the tactic's radio without the world (F-26639 after the tactic update's countdown, F-63286): the line
/// said (its scenario name, the sender's index in the cops, the receiver's index; none = DISPATCH).
pub(crate) fn chatter_step(st: &mut RdoState, i: &RdoIn) -> Option<(&'static str, usize, Option<usize>)> {
    st.t = (st.t - i.dt).max(0.0);
    let n = i.n;
    if !i.on || i.busy || n == 0 {
        return None;
    }
    let mut call = None;
    if i.target {
        // A: on to or off a freeway: the road the target's lane belongs to, stable for 1 s; said once, on the expiry frame
        if let (true, true, Some(r)) = (i.is_player, i.rec, i.road) {
            if st.road == Some(r) {
                if st.fw_t > 0.0 {
                    st.fw_t -= i.dt;
                    if st.fw_t <= 0.0 {
                        let k = rdo_cop(&mut st.rng, n);
                        call = Some((if (i.is_fw)(r) { "REPORT_FREEWAY_ENTER" } else { "REPORT_FREEWAY_EXIT" }, k, None));
                    }
                }
            } else {
                if (i.is_fw)(r) != st.road.is_some_and(|o| (i.is_fw)(o)) {
                    st.fw_t = 1.0;
                }
                st.road = Some(r);
            }
        }
        // B: a police car hit hard, from the cop whose entry is the largest (the first of equals; floor -1)
        if call.is_none() && i.rec && i.hard {
            let mut best = -1.0f32;
            let mut who = None;
            for (k, v) in i.hit.iter().enumerate().take(n) {
                if let Some(v) = *v {
                    if v > best {
                        best = v;
                        who = Some(k);
                    }
                }
            }
            if let Some(k) = who {
                call = Some(("REPORT_PURSUIT_HIT_COP_CAR_HARD", k, None));
            }
        }
        // C: the nitrous seen, twice at most a tactic
        if call.is_none() && st.nitro < 2 && n > 1 && i.nitro_burning {
            let k = rdo_cop(&mut st.rng, n);
            call = Some(("REPORT_UPGRADE_NITROUS", k, Some((k + 1) % n)));
            st.nitro += 1;
        }
    }
    // D: the periodic line, a uniform pick of those that may be said
    if call.is_none() && st.t == 0.0 {
        let k = rdo_cop(&mut st.rng, n);
        let mut list: Vec<(&'static str, bool)> = vec![("REPORT_LOCATION_UPDATE", false), ("REPORT_IDENTIFY_DRIVER", false)];
        if i.mgr80 == 1 {
            list.push(("REPORT_REQUEST_BACKUP", false));
        }
        list.push(("REPORT_SUSPECT_DESTRUCTION", false));
        if i.speed > 44.7 {
            list.push(("REPORT_SUSPECT_SPEED", false));
        }
        if n > 1 {
            list.push((if i.in_race { "REPORT_RACE_OFFICER_TO_OFFICER" } else { "REPORT_CRUISE_OFFICER_TO_OFFICER" }, true));
            list.push(("REPORT_PURSUIT_STATEMENT", true));
            list.push(("REPORT_UPGRADE_MISC", true));
            // (F-29443: a cop within the distance; the far byte: said once each time the cops come in from beyond it)
            if !i.within_vic {
                st.far = true;
            } else if !i.within_10 && st.far {
                st.far = false;
                list.push(("REPORT_MANEUVER_VICINITY", false));
            }
        }
        let (name, other) = list[rdo_draw(&mut st.rng) as usize % list.len()];
        call = Some((name, k, other.then(|| (k + 1) % n)));
    }
    // E: any line sets the countdown again, refused by its Frequency or not
    if call.is_some() {
        let x = rdo_draw(&mut st.rng) & 0x7F_FFFF;
        st.t = i.min_t + (i.max_t - i.min_t) * x as f32 * 2f32.powi(-23);
    }
    call
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
    /// the tactic's type (vt+56): 0 Dispatch to Location, 1 Pull Over, 2 Pursuit, 3 Double Pursuit, 4 Arrow Trap; -1 none
    fn ty(self) -> i32 {
        match self {
            TacKind::None => -1,
            TacKind::Dispatch => 0,
            TacKind::PullOver => 1,
            TacKind::Pursuit => 2,
            TacKind::PursuitDbl => 3,
            TacKind::ArrowTrap => 4,
        }
    }

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
    /// the state asked for (true Halt, false CatchUp), entered at the start of the next update
    next: Option<bool>,
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
    /// the brain's route is set; it has arrived (brain +576 == 1, latched by GoToPosition F-16378)
    routed: bool,
    arrived: bool,
    /// the brain's +580 (within 8 m of its goal, F-74140) and its GoToPosition state's time (+8, from the command)
    near_t: f32,
    go_t: f32,
    /// a state asked for (FSM +16, F-28114), entered at the start of the next FSM update (F-43677). (The machine is begun
    /// in state 0 by the engage itself, F-11097 / F-26187, as the dispatched cop is a car already in the world - loaded)
    next: Option<u8>,
    /// the cop's brain words the commands write (Respond 8, the search 7): the tactic entries are cleared while no
    /// pursuit runs, so the dispatch keeps its cop's own; handed to the Pursuit it turns into (the brain is the cop's)
    brain: BrainCmd,
}

/// A race launcher's EvaluatePolice arguments (docs/reference/hookmen/evaluate_police.md): the chance, the gate
/// "more than 5 racers - chance 0" of the hookman / story cores and RaceWrapper, the chase properties and the chase type.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PoliceEval {
    pub chance: f32,
    pub gate5: bool,
    pub props: [bool; 3],
    pub chase_type: u8,
    /// SetRaceChaseTarget's racer (manager +208) is the player
    pub chase_player: bool,
}

impl PoliceEval {
    /// chance 0: wager / pink slip, delivery, time trial, tournament, series, red light, seven story cores
    pub const NEVER: PoliceEval = PoliceEval { chance: 0.0, gate5: false, props: [true; 3], chase_type: race_chase_type(0, None), chase_player: false };
    /// 25 % with the racers gate: generic_local_hookman, the *_CHAMP_CORE and BEAT_SH_0x cores, RaceWrapper
    pub const GATED: PoliceEval = PoliceEval { chance: 0.25, gate5: true, props: [true; 3], chase_type: race_chase_type(0, None), chase_player: false };
    /// 25 %, no gate: TelephoneChallenge
    pub const PHONE: PoliceEval = PoliceEval { chance: 0.25, gate5: false, props: [true; 3], chase_type: race_chase_type(0, None), chase_player: false };
    /// BEAT_SH_04B_CORE: always, the player the zone's target, no tracking, no race retarget
    pub const SH_04B: PoliceEval =
        PoliceEval { chance: 1.0, gate5: false, props: [false, false, true], chase_type: race_chase_type(1, Some(true)), chase_player: true };
}

/// The race's cop zone (PoliceManager +188 zone, +192 lap, +196 state, +212 timer).
#[derive(Clone, Debug)]
pub struct CopZone {
    pub idx: usize,
    pub lap: usize,
    /// 0 waiting for its lap, 1 armed, 2 set off (5.5 s), 3 the radio delay (fCopZoneRdoDelayTime)
    pub state: u8,
    pub timer: f32,
    /// the target (+200): its car's uid, and whether it is the player
    pub target: Option<u32>,
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
    /// where it comes up (+912): its home, then the start point of the patrol path it was given (F-55649)
    pub at: Vec3,
    /// its patrol path (the group) and the point of it the route is heading for
    pub path: Option<usize>,
    pub pi: usize,
    /// +1128: the reassign's cooldown
    pub cd: f32,
    /// +1156: the next assign may move it (out of sight; 0 after a successful assign)
    pub relocate: bool,
    /// the random Follow (cmd 2, F-24671): following the player (+1076), its roll / down timer +1104, the 1 s timers
    /// +1108 (the racer behind) / +1112 (facing away) / +1116 (moving away) and the last goal +1088
    pub follow: bool,
    pub fol_t: f32,
    pub fol_tm: [f32; 3],
    pub fol_goal: Option<Vec3>,
    /// +1073: may patrol (off under SetAllPoliceActive(0) and after a failed assign; on again at the end of an untimed race)
    pub may_patrol: bool,
    /// +1072: may check law breaking - written with +1073 by every writer (the unit reset F-82662 / F-22895 1, underground
    /// F-97540 !a, SetAllPoliceActive F-75974, Engage's F-97878 1, a failed assign 0) except the patrol loop's race-end and
    /// [8286E800] stores of +1073 alone. Read by the offence check F-52240 and the patrol loop F-24233 (the second-cop
    /// upgrade and the Follow): a unit without it detects nothing and follows nobody.
    pub may_check: bool,
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
    /// a career speech line plays (the speech manager's +2256 and +2260; main)
    pub speech_busy: bool,
    pub in_race: bool,
    /// +136: while above 0 the brains' lost flag +773 is off (F-79519; F-63286 counts it down and sets the flag
    /// again); the tactic's begin (F-82656) sets fTrackTgtTime - the manager's track byte is 1 by default (F-39756)
    track_t: f32,
    /// the brains' lost flag +773 (the tactic's cops all alike): F-79519 clears it (the begin, SetChaseProperties, a
    /// re-sighting in the Search), the base update F-63286 sets it when +136 runs down past 0 and the tactic's vt+28
    /// answers (the Pursuit / Double Pursuit / Arrow Trap: +29, the first chase property; the pull-over: 0), the end
    /// F-67316 sets it; the loss tests need it (F-68032: +773 && unseen > fLostTgtThresholdTime)
    lost_ok: bool,
    /// the cops whose siren target the last tactic end set to 0 (F-67316 with its flag, not arrested): taken by the next
    /// siren step
    lamps_off: Vec<u32>,
    /// the lead's brain: unseen time +596, the time in the search cone +600, the last known velocity +624
    last_known_v: Vec3,
    /// each cop brain's +624: the target's velocity at its last sight-tracker tick (F-54688 - not under command 3 Pull Over
    /// or 8 Go To Position, nor in the search); zero for a fresh brain (F-52769)
    brain_tvel: HashMap<u32, Vec3>,
    /// the searchers' brain state 7
    srch: Vec<CopSearch>,
    prev_heat: Heat,
    /// a line for the police radio from the search (F-40917): REPORT_SUSPECT_LOST when the clock starts, _SPOTTED or
    /// _REACQUIRE when it ends with a sighting after 2 s
    pub radio_cue: Vec<String>,
    /// F-52240's ladder started a tactic from a detecting cop (no tactic yet): the speech line it requests (F-70230)
    /// before StartTactic - Some(the tactic's AI target, None = the player); main picks PLAYER_COPS_BEGIN_PURSUIT or the
    /// race taunt
    pub begin_cue: Option<Option<u32>>,
    /// the report of this update's Event::Pursuit when it is not REPORT_VIOLATION_PURSUIT (F-52240's hit-a-cop steal:
    /// REPORT_HIT_COP_CAR_HARD, the report F-61876 makes); cleared at every update
    pub start_report: Option<&'static str>,
    /// the start's default report's +8 (F-52240: the law record's +100 & +96 - this update's offence bits, the
    /// original's numbering) when the ladder started the tactic; None for the other starts (+8 = 0)
    pub report_bits: Option<u32>,
    /// this update's offence bits in the original's numbering (the record's +96, the bits F-14569 reads)
    tick_law: u32,
    arrest: f32,
    /// the tactic's +140 (the target burning out) and +144 (barely moving) timers and +48, the target's last place
    t_burn: f32,
    t_still: f32,
    prev_pos: Option<Vec3>,
    /// the Pursuit's Delay (`flee_hold`) ran last frame: its exit F-25831 zeroes +108, +140, +144
    arrest_delay: bool,
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
    /// the cop zone's target when it is an AI racer (its uid): the chase on it starts at the next update
    zone_target: Option<u32>,
    /// the tactic's target (+32) when it is an AI racer (its uid); None: the player. An AI target has no law record
    /// (F-50742 gives one only to the player), is never arrested (every tactic's Arrest / Ticket ask needs the
    /// target's vt+40, the player), and its tactic ends by losing it or by the race (docs/reference/hookmen/cop_zone.md)
    pub target: Option<u32>,
    /// the race's racers (uids, the player first) for the in-race retarget (F-22530); set by main.rs each frame
    pub racer_uids: Vec<u32>,
    /// the zone of the race running was picked
    pub zone_picked: bool,
    /// the launching script's EvaluatePolice for the race about to run (set at the race's start, taken on its first frame)
    pub pending_eval: Option<PoliceEval>,
    /// Registry nNumNoCopRaces: races in a row whose roll gave no police (session only, a missing key reads 0)
    pub no_cop_races: u32,
    /// the last EvaluatePolice destroyed every police car: no police while that race runs (and its restarts)
    pub race_off: bool,
    /// PoliceManager_SetChaseProperties (manager +220 / +221 / +222): a new tactic's +29 track the target, +30 may
    /// retarget in a race, +31; and the running tactic's +30
    chase_props: [bool; 3],
    tac_retarget: bool,
    /// PoliceManager_SetRaceChaseTarget's type (manager +204): 0 the zone's roll, 1 the racer that set it off, 2 the
    /// nearest AI racer
    chase_type: u8,
    /// its racer +208 is the player (BEAT_SH_04B): the zone's target whoever sets it off
    chase_player: bool,
    /// the pull-over's bullhorn (its tactic +92 = 2 at iBullhornLawBreakScore, F-22530): seconds left of
    /// fBullhornTime; Some(0) = run out (the 599 cap is lifted)
    bullhorn: Option<f32>,
    /// the ticket hold (FsmPullOver StateTicket, F-25878 / F-46174): (the state's time, its limit): the tick sets the
    /// result once the time - before this update's dt is added - is over the limit, then COP_PULLOVER
    ticket_hold: Option<(f32, f32)>,
    /// COP_PULLOVER is running (CopPullover.sco): the police wait for `ticket_done` or `suspect_fled`
    ticketing: bool,
    /// after a tactic on the player ends: no scoring for 5 s (the record's +116, F-22530 / F-54211)
    grace: f32,
    /// the pursuit after a bolt from the ticket starts with its cruisers held this long (the Pursuit
    /// tactic's Delay state, F-10824 / F-50798: request +20 = the bolt's time x fPullOverFleeDelayScale)
    flee_hold: f32,
    /// the Delay state's time (its +8, from 0 at the enter) and its ask of CatchUp (the FSM's +16), applied at the next update
    flee_t: f32,
    flee_ask: bool,
    /// each pursuing cruiser's tactic state (by car uid), and the target's speed at the last update (tactic +64)
    tacs: HashMap<u32, CopTac>,
    last_tspeed: f32,
    /// the tactic's +88: the cop whose car the target's seen probe hit nearest (F-16046; kept while the target is not
    /// moving, the probe then not run); the common update F-63286 brakes it (F-39048, 1.0)
    probe_cop: Option<u32>,
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
    wrong_active: bool,
    burnout_active: bool,
    /// block traffic's started bit (F-88545)
    block_active: bool,
    /// the law record's wrong-way bit (0x2000 of its frame offence bits, F-88545: the condition F-29334 held for
    /// fWrongWayThresholdTime, every tick once started); kept through the grace, as the record is
    against_traffic: bool,
    /// the walkers' feet (`Peds::capsule_feet`), set by the caller each frame: their capsules are in the seen / block probe
    pub peds: Vec<Vec3>,
    last_damage: f32,
    spawn_timer: f32,
    /// how long the running pursuit has lasted (seconds)
    pub chase_time: f32,
    last_known: Vec3,
    /// time until the next radio report of the target's position
    /// the tactic's radio (F-26639): the countdown +112, the law record's road +60 and its timer +64, the nitrous tries
    /// +100, the far byte +96, this frame's hard-hit table, the line's draw
    rdo_t: f32,
    rdo_road: Option<usize>,
    rdo_fw_t: f32,
    rdo_nitro: u8,
    rdo_far: bool,
    hard_tab: Option<Vec<(u32, f32)>>,
    rdo_rng: (u32, u32),
    time: f32,
    rng: u32,
    /// the launching scripts' generator for EvaluatePolice's roll (a script context's MWC, seeded when first drawn from)
    script_rng: Option<Mwc>,
    glow_buf: Option<wgpu::Buffer>,
    glow_count: u32,
    pub enabled: bool,
    /// per tactic cop: the teleport cooldown (+1132) and its brain's time off the nav mesh (+584, F-74140)
    tel: HashMap<u32, (f32, f32, f32)>,
    /// the target's last nav road is a freeway or a ramp (law record +60, F-28783: its name starts "FW" / "XT")
    fw_road: bool,
    /// the tactic's +120 (no teleport while above 0) and its duration +104 (F-63286)
    tel_lock: f32,
    tac_t: f32,
    /// the connection-list order flag of F-92039 (flips after every growth)
    grow_flip: bool,
    /// SetAllPoliceUnderground(1, 1) is in force: no unit comes up or patrols until (0, 1)
    pub units_held: bool,
    /// a race with no time limit is current (F-24233: race current and race +3148 == 0), and last frame's (mgr +88):
    /// at its end every free unit may patrol again. (F-48953 also locks the free units in the streaming during it: the
    /// port keeps every unit's car in the world anyway)
    pub untimed_race: bool,
    untimed_prev: bool,
    /// junction the player was last seen approaching on red
    /// the junction of the car's nearest lane when it last entered one (law record +152, F-65840)
    last_junction: Option<usize>,
    /// step 19d: the pursuit under way is the payback client's friends' (`start_goons`):
    /// their cars (`CarInstance::goon`) are the pursuers, no cruiser is sent, nothing is
    /// lit, and the two results are the payback leg's to deal with
    pub goons: bool,
    /// a tactic of kind 1 - 4 started (StartTactic F-33550 calls the phone's interrupt F-51256: the call on the
    /// phone is put away and dropped); the main loop takes it
    pub phone_cut: bool,
    /// StartTactic (F-33550) asked for the pursuit tutorial (F-24639): a Pursuit started on the player, or a Double
    /// Pursuit / Arrow Trap started with no tactic before it (one that ended - a Dispatch, a Pull-over - gives none);
    /// the main loop takes it
    pub pursuit_tut: bool,
    /// The police strip's texts StartTactic asks for (F-76838's message kinds: 0 pull over, 1 pursuit, 2 investigation,
    /// 3 searching), on every start, whatever the target (every caller passes the show flag 1); main hands them to the HUD
    pub hud_msgs: Vec<u8>,
    /// The manager's +84 (PoliceManager_GetLastPullOverPolice, F-66567): the first cop of the last pull-over started; never
    /// cleared
    pub last_pullover: Option<u32>,
    /// Starts to count in the stat gCPA ("cops activated"; StartTactic's stats manager vt+48: add 1, sub 0), taken by the game
    pub stat_cpa: u32,
    /// the player's car's vehicle-table class is the police's (F-96166: the camera target's car, F-10499 = 5): the law
    /// score step F-52240 does not run (no offences, no score, no score ladder) and an armed cop zone is not set off
    /// (F-71212's state 1). Set by the main loop each frame.
    pub player_police: bool,
    /// the NO FINES cheat (the byte F-60274 toggles, cleared by F-72804): the fine routine F-83946 returns 0 at once.
    /// Set by the main loop each frame.
    pub no_charges: bool,
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
const LAW_BLOCK_TRAFFIC: u32 = 1024;
/// the hard hit on a police car (the original's 0x20, set only with 0x40 = `LAW_POLICE_VEHICLE`: F-93844, a cop value
/// of the contact table over 22.35)
const LAW_POLICE_HARD: u32 = 2048;
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
            t_burn: 0.0,
            t_still: 0.0,
            prev_pos: None,
            arrest_delay: false,
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
            speech_busy: false,
            in_race: false,
            track_t: 0.0,
            lost_ok: true,
            lamps_off: Vec::new(),
            last_known_v: Vec3::ZERO,
            brain_tvel: HashMap::new(),
            srch: Vec::new(),
            prev_heat: Heat::None,
            radio_cue: Vec::new(),
            begin_cue: None,
            start_report: None,
            report_bits: None,
            tick_law: 0,
            arrest_over: false,
            zone: None,
            zone_go: false,
            zone_target: None,
            target: None,
            racer_uids: Vec::new(),
            zone_picked: false,
            pending_eval: None,
            no_cop_races: 0,
            race_off: false,
            chase_props: [true; 3],
            tac_retarget: true,
            chase_type: 0,
            chase_player: false,
            tacs: HashMap::new(),
            last_tspeed: 0.0,
            probe_cop: None,
            bullhorn: None,
            ticket_hold: None,
            ticketing: false,
            grace: 0.0,
            flee_hold: 0.0,
            flee_t: 0.0,
            flee_ask: false,
            speed_level: 0,
            speed_top: 0,
            sec: [0.0; 7],
            off_active: false,
            two_active: false,
            jump_paid: false,
            fled: false,
            progress: 1.0,
            wrong_active: false,
            burnout_active: false,
            block_active: false,
            against_traffic: false,
            peds: Vec::new(),
            last_damage: 0.0,
            spawn_timer: 8.0,
            chase_time: 0.0,
            last_known: Vec3::ZERO,
            rdo_t: 0.0,
            rdo_road: None,
            rdo_fw_t: 0.0,
            rdo_nitro: 0,
            rdo_far: true,
            hard_tab: None,
            rdo_rng: (0x2f6e_2b1d, 0x51ed_270b),
            time: 0.0,
            rng: 0x51ed_270b,
            script_rng: None,
            glow_buf: None,
            glow_count: 0,
            enabled: true,
            tel: HashMap::new(),
            fw_road: false,
            tel_lock: 0.0,
            tac_t: 0.0,
            grow_flip: true,
            units_held: false,
            untimed_race: false,
            untimed_prev: false,
            last_junction: None,
            goons: false,
            phone_cut: false,
            pursuit_tut: false,
            hud_msgs: Vec::new(),
            last_pullover: None,
            stat_cpa: 0,
            player_police: false,
            no_charges: false,
            units: Vec::new(),
        }
    }

    pub fn load(&mut self, data_dir: &str) {
        let dir = Path::new(data_dir).join("tune/police");
        self.tun = Tuning::load(&dir.join("globals.xml"));
        self.safe_zones = load_safe_zones(Path::new(data_dir));
        // (the manager's constructor loads `<city prefix>_patrolpath` - "sc", 15 paths - and `patrolpath` only when that fails;
        // an older install has the first under its name hash)
        self.paths = ["sc_patrolpath.xml", "0x4f283eb3", "patrolpath.xml"].iter().map(|n| dir.join(n)).find(|p| p.is_file()).map_or(Vec::new(), |p| load_patrol_paths(&p));
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
            self.units.push(Unit { id, home: Vec3::from(*home), heading: *heading, slot: Vec3::from(*slot), under: true, at: Vec3::from(*home), path: None, pi: 0, cd: 0.0, relocate: true, follow: false, fol_t: 0.0, fol_tm: [0.0; 3], fol_goal: None, may_patrol: true, may_check: true });
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

    /// PoliceManager_SetAllPoliceUnderground(1, 1) (F-24122 -> F-47166 / F-97540): the manager's record cleared, then
    /// every unit with +872 == 1 - all three, wherever they are - ends its tactic or command and is put at its slot
    /// with no speed and no map icon; the port takes their cars off the road.
    pub fn units_down(&mut self, vehicles: &mut Vehicles) {
        for u in self.units.iter_mut() {
            u.under = true;
            u.may_patrol = false;
            u.may_check = false;
        }
        self.units_held = true;
        // (an underground unit has no car on the road, and so none for IsAnyPoliceCloseToPlayer,
        // F-91297, which skips the police whose underground flag +880 is set)
        vehicles.cars.retain(|c| !(c.cop && c.vinyl_name.starts_with("police_unit:")));
    }

    /// CopArrest.sco's 0xaa8 SetAllPoliceUnderground(1, 1), before the scene's slots: every unit underground as
    /// `units_down`, but the cars the scene cast (Cars_AssignIdToSlot then the mover snap bring them from the slot place
    /// into the scene; the port keeps them where the scene posed them) stay; the rest leave the road now, not after the scene.
    pub fn units_down_but_cast(&mut self, vehicles: &mut Vehicles) {
        for u in self.units.iter_mut() {
            u.under = true;
            u.may_patrol = false;
            u.may_check = false;
        }
        self.units_held = true;
        vehicles.cars.retain(|c| !(c.cop && c.vinyl_name.starts_with("police_unit:") && !c.driver.starts_with("cine:")));
    }

    /// PoliceManager_SetAllPoliceUnderground(0, 1): the units come up at their surface place;
    /// the port puts each there (`update_units`) while the player is within streaming reach.
    pub fn units_up(&mut self) {
        // (SetUnderground(0) first puts the car at its +912: the patrol loop brings each up there; F-97540: flags = !a)
        self.units_held = false;
        for u in self.units.iter_mut() {
            u.may_patrol = true;
            u.may_check = true;
        }
    }

    /// The units that are up stand at their home as the script made them (heading in degrees -
    /// the port's yaw is the heading in radians) while the player is within 380 m; a unit whose
    /// car has gone counts as underground again.
    pub fn update_units(&mut self, _gpu: &Gpu, _world: &World, vehicles: &mut Vehicles) {
        // (a unit whose car has gone - removed by a scene, a reset - counts as underground; the patrol loop, F-24233,
        // brings the units up and moves them)
        for u in self.units.iter_mut() {
            let tag = format!("police_unit:{}", u.id);
            if !u.under && !vehicles.cars.iter().any(|c| c.cop && c.vinyl_name == tag) {
                u.under = true;
            }
        }
    }

    /// The car the police scripts put in a police scene's car slot `slot` (Cars_AssignIdToSlot): CopArrest.sco 0x357 /
    /// 0xa1f slot k = unit 400 + k - 1 (FindFromUID(400) / (401)); CopPullover.sco 0x4b2 slot 1 = the racer of
    /// GetLastPullOverPolice, else unit 400. None for any other scene (not a police script's); Some(None) when that car is
    /// not on the road (an underground unit is brought from its slot place by the scene's mover).
    pub fn scene_cop(&self, vehicles: &Vehicles, scene: &str, slot: u8) -> Option<Option<usize>> {
        if slot == 0 {
            return None;
        }
        let unit = |k: u32| Self::unit_car(vehicles, 400 + k);
        if scene.eq_ignore_ascii_case("cut_cop_walk_02") && slot == 1 {
            let last = self.last_pullover.and_then(|u| vehicles.cars.iter().position(|c| c.uid == u && c.cop));
            return Some(last.or_else(|| unit(0)));
        }
        if scene.to_ascii_lowercase().starts_with("cut_cop_arrest") {
            return Some(unit(slot as u32 - 1));
        }
        None
    }

    /// A unit's car (index) when it is up.
    fn unit_car(vehicles: &Vehicles, id: u32) -> Option<usize> {
        let tag = format!("police_unit:{id}");
        vehicles.cars.iter().position(|c| c.cop && c.vinyl_name == tag)
    }

    /// Bring unit `ui` up at `q` facing `fwd` (SetUnderground(0) + F-86308: at the matrix, zero velocity); its car's uid.
    fn unit_surface(&mut self, gpu: &Gpu, world: &World, vehicles: &mut Vehicles, ui: usize, q: Vec3, fwd: Vec3) -> Option<u32> {
        let model = vehicles.models.iter().find(|m| m.name == COP_CAR).cloned()?;
        if !world.ready_at(q) {
            return None;
        }
        let idx = vehicles.spawn(gpu, model, q, (-fwd.x).atan2(-fwd.z), [0.92, 0.92, 0.92, 0.0], false);
        Self::dress_cruiser(world, vehicles, idx);
        let c = &mut vehicles.cars[idx];
        c.cop = true;
        c.vinyl_name = format!("police_unit:{}", self.units[ui].id);
        let rail_v = world.nav.as_ref().and_then(|n| Some(crate::vehicles::traffic::rail_speed(n, n.nearest_link(q)?.0).0)).unwrap_or(crate::mc4::RAIL_SPEED);
        if let Some(ai) = c.ai.as_mut() {
            ai.target_speed = rail_v;
        }
        self.units[ui].under = false;
        Some(c.uid)
    }

    /// The patrol loop (F-24233), each manager frame, for every unit in no tactic: underground while the police are
    /// off; a unit with no patrol path is given the one nearest the player (F-90720, free paths only, moved out of
    /// sight when +1156 is set); a patrolling unit beyond fCopHudIconVisibleDist of the player is given one again and
    /// moved (F-79523; a taken path: 5 s before the next try); a 1 % roll (iChangePathProbPrcnt, F-84839) restarts its
    /// route from the nearest point; then it drives its path, round and round.
    fn patrol(&mut self, gpu: &Gpu, world: &World, vehicles: &mut Vehicles, tun: &Tuning, busy: &[u32], dt: f32) {
        let Some(pp) = vehicles.player().map(|c| c.state.pos) else { return };
        let vis = tun.get("fCopHudIconVisibleDist", 800.0);
        let change = tun.get("iChangePathProbPrcnt", 1.0) as i32;
        let race_over = self.untimed_prev && !self.untimed_race;
        self.untimed_prev = self.untimed_race;
        for ui in 0..self.units.len() {
            let car = Self::unit_car(vehicles, self.units[ui].id);
            if car.is_some_and(|i| busy.contains(&vehicles.cars[i].uid)) {
                // (a tactic's command replaces the follow's)
                self.units[ui].follow = false;
                continue;
            }
            if !self.enabled {
                if let Some(i) = car {
                    vehicles.cars.remove(i);
                }
                self.units[ui].under = true;
                continue;
            }
            if self.units_held {
                continue;
            }
            if race_over {
                self.units[ui].may_patrol = true;
            }
            if !self.units[ui].may_patrol {
                self.units[ui].follow = false;
                continue;
            }
            match self.units[ui].path {
                None => {
                    let r = self.assign(gpu, world, vehicles, ui, true, self.units[ui].relocate, vis);
                    if r != 0 && std::env::var_os("MCLA_PATROL_TRACE").is_some() {
                        log::info!("police: unit {} assign failed ({})", self.units[ui].id, r);
                    }
                    if r == 0 {
                        self.units[ui].relocate = false;
                        // (AssignPath, F-55649 -> F-30531: +1128 = fPatrolTime, the hold before the next reassign)
                        let g = self.units[ui].path.map_or(-1, |g| g as i32);
                        self.units[ui].cd = patrol_hold(g, self.units[ui].cd, tun.get("fPatrolTime", 5.0));
                    } else if let Some(i) = car.filter(|_| !self.units[ui].under) {
                        // (F-24233: any failure of F-90720 on the surface - 2 no path, 3 no controller, 4 group
                        // taken - sends it underground with +1072 = +1073 = 0, no patrol until something allows it again;
                        // an underground unit's failure changes nothing, it tries again next frame)
                        vehicles.cars.remove(i);
                        self.units[ui].may_patrol = false;
                        self.units[ui].may_check = false;
                        self.units[ui].under = true;
                    }
                    continue;
                }
                Some(_) if self.units[ui].under => {
                    // (SetAllPoliceUnderground(0, 1): up at +912)
                    let u = &self.units[ui];
                    let (at, fwd) = match u.path.and_then(|g| self.paths.get(g)) {
                        Some(p) if !p.is_empty() => (u.at, (p[(u.pi) % p.len()] - u.at).normalize_or(Vec3::X)),
                        _ => (u.at, Vec3::X),
                    };
                    if self.unit_surface(gpu, world, vehicles, ui, at + Vec3::Y * 0.8, fwd).is_some() {
                        log::info!("police: unit {} up at {:?} (its +912)", self.units[ui].id, at);
                    }
                    continue;
                }
                Some(_) => {}
            }
            let Some(i) = car else { continue };
            let pos = vehicles.cars[i].state.pos;
            // (the cop's per-frame F-79540: +1104 counts down to 0)
            self.units[ui].fol_t = (self.units[ui].fol_t - dt).max(0.0);
            // (F-24233: a unit without +1072 starts no Follow and leaves the one it is in, command 0)
            let following = if self.units[ui].may_check {
                self.follow_step(world, vehicles, tun, ui, i, dt)
            } else {
                std::mem::replace(&mut self.units[ui].follow, false)
            };
            if following {
                if !self.units[ui].follow {
                    // (F-69094's end: the patrol loop finds the brain idle and assigns the patrol path again, F-90720
                    // without moving it)
                    if self.assign(gpu, world, vehicles, ui, true, false, vis) == 0 {
                        let g = self.units[ui].path.map_or(-1, |g| g as i32);
                        self.units[ui].cd = patrol_hold(g, self.units[ui].cd, tun.get("fPatrolTime", 5.0));
                    } else if let Some(path) = self.units[ui].path.and_then(|g| self.paths.get(g)) {
                        self.units[ui].pi = nearest_point(path, pos);
                    }
                }
                continue;
            }
            self.units[ui].cd -= dt;
            if self.units[ui].cd <= 0.0 && pos.distance(pp) > vis {
                // (assigned: AssignPath's fPatrolTime hold; the path taken: 5 s before the next try)
                match self.assign(gpu, world, vehicles, ui, false, true, vis) {
                    0 => {
                        let g = self.units[ui].path.map_or(-1, |g| g as i32);
                        self.units[ui].cd = patrol_hold(g, self.units[ui].cd, tun.get("fPatrolTime", 5.0));
                    }
                    4 => self.units[ui].cd = 5.0,
                    _ => {}
                }
                continue;
            }
            // (F-84839: the port's paths are the groups' paths - each group of the disc's file holds one, so the group is
            // that one path and nothing changes; the general rule is `path_change`)
            if let Some(g) = self.units[ui].path {
                let group = std::slice::from_ref(&self.paths[g]);
                if let Some((j, k)) = path_change(group, 0, &mut self.units[ui].cd, pos, change, &mut self.rdo_rng) {
                    if j >= 0 {
                        self.units[ui].pi = k as usize;
                    }
                }
            }
            // its route: the path's points in turn, wrapping (mcBrainCruising with looping waypoints and no speed of its
            // own: the rail's speed, the port's driver steers between the points)
            let Some(path) = self.units[ui].path.and_then(|g| self.paths.get(g)).filter(|p| !p.is_empty()) else { continue };
            let mut k = self.units[ui].pi % path.len();
            if pos.distance(path[k]) < 15.0 {
                k = (k + 1) % path.len();
                self.units[ui].pi = k;
            }
            let rail_v = world.nav.as_ref().and_then(|n| Some(crate::vehicles::traffic::rail_speed(n, n.nearest_link(pos)?.0).0)).unwrap_or(crate::mc4::RAIL_SPEED);
            if let Some(ai) = vehicles.cars[i].ai.as_mut() {
                ai.chase = Some((path[k], Vec3::ZERO));
                ai.chase_exact = false;
                ai.target_speed = rail_v;
            }
            if std::env::var_os("MCLA_PATROL_TRACE").is_some() && (self.time * 30.0) as u32 % 150 == 0 {
                log::info!("police: unit {} patrols at {:?} speed {:.1}, point {} of path {:?}, {:.0} m from the player", self.units[ui].id, pos, vehicles.cars[i].state.speed(), k, self.units[ui].path, pos.distance(pp));
            }
        }
    }

    /// The random Follow (F-24233's start and break-off rolls, cmd 2 F-24671, its tick F-68135, the end F-69094), for
    /// a patrolling unit's car `i`. Start, while +1104 is 0 and the player is no tactic's target and no other unit
    /// follows him: the cop sees him within fDetectDist (40 m) in a 45 degree half cone (F-94676, a line 2 m up),
    /// he is ahead of the cop, both face and move the same way; then an iRndFollowOnProbPrcnt (5 %) roll, failed: +1104
    /// = 1 s. Following: +1104 = fFollowDownTime (5 s); it ends after 1 s of the player behind the cop, facing or moving
    /// away, beyond fStopFollowDist (500 m), with no rail within 5 m of the goal, or on an iRndFollowOffProbPrcnt (8 %)
    /// roll each time +1104 runs out (failed: 1 s). The goal: 10 m (fGoalFollowDist) behind the player's lane point
    /// along his row 2, else behind the player, on the nearest rail within 5 m; re-planned when it moved over 1 m. The
    /// end sets +1104 = fFollowDownTime. True while it follows (and in the frame it ends). (The player is the only
    /// racer; the port's driver steers to the goal at the rail's speed for the brain's traffic ghost.)
    fn follow_step(&mut self, world: &World, vehicles: &mut Vehicles, tun: &Tuning, ui: usize, i: usize, dt: f32) -> bool {
        let Some(p) = vehicles.player() else { return false };
        let (ppos, pvel, prow2) = (p.state.pos, p.state.vel, -p.state.forward());
        let lane_pt = vehicles.driver_lanes.get(&p.uid).and_then(|d| Some((d.best?.0, d.best_t))).and_then(|(l, t)| Some(world.nav.as_ref()?.curves.get(l)?.eval(t).0));
        let c = &vehicles.cars[i];
        let (cpos, cvel, crow2) = (c.state.pos, c.state.vel, -c.state.forward());
        let d = ppos - cpos;
        let down = tun.get("fFollowDownTime", 5.0);
        if !self.units[ui].follow {
            if self.units[ui].fol_t > 0.0 || self.kind != TacKind::None || self.units.iter().any(|u| u.follow) {
                return false;
            }
            let ok = cop_sees(world, c, ppos, tun.get("fDetectDist", 40.0), 45.0) && crow2.dot(d) < 0.0 && crow2.dot(prow2) > 0.0 && cvel.dot(pvel) > 0.0;
            if !ok {
                return false;
            }
            if self.rand() < tun.get("iRndFollowOnProbPrcnt", 5.0) * 0.01 {
                log::info!("police: unit {} randomly starting to follow the player", self.units[ui].id);
                let u = &mut self.units[ui];
                u.follow = true;
                u.fol_t = down;
                u.fol_tm = [0.0; 3];
                u.fol_goal = None;
            } else {
                self.units[ui].fol_t = 1.0;
                return false;
            }
        }
        let conds = [crow2.dot(d) > 0.0, crow2.dot(prow2) < 0.0, cvel.dot(pvel) < 0.0];
        let mut end = false;
        for (t, c) in self.units[ui].fol_tm.iter_mut().zip(conds) {
            *t = if c { *t + dt } else { 0.0 };
            end |= *t > 1.0;
        }
        end |= d.length() > tun.get("fStopFollowDist", 500.0);
        let goal = if end {
            None
        } else {
            let back = tun.get("fGoalFollowDist", 10.0);
            let snap = |q: Vec3| {
                let nav = world.nav.as_ref()?;
                let (l, t, dd) = nav.nearest_link(q)?;
                (dd <= 5.0).then(|| (nav.links[l].point_at(t), crate::vehicles::traffic::rail_speed(nav, l).0))
            };
            lane_pt.and_then(|q| snap(q + prow2 * back)).or_else(|| snap(ppos + prow2 * back))
        };
        end |= goal.is_none();
        if !end && self.units[ui].fol_t <= 0.0 {
            if self.rand() < tun.get("iRndFollowOffProbPrcnt", 8.0) * 0.01 {
                log::info!("police: unit {} randomly breaking off the follow", self.units[ui].id);
                end = true;
            } else {
                self.units[ui].fol_t = 1.0;
            }
        }
        if end {
            let u = &mut self.units[ui];
            u.follow = false;
            u.fol_t = down;
            u.fol_goal = None;
            return true;
        }
        let (g, v) = goal.unwrap();
        if self.units[ui].fol_goal.map_or(true, |q| q.distance(g) > 1.0) {
            self.units[ui].fol_goal = Some(g);
        }
        let g = self.units[ui].fol_goal.unwrap_or(g);
        if let Some(ai) = vehicles.cars[i].ai.as_mut() {
            ai.chase = Some((g, Vec3::ZERO));
            ai.chase_exact = false;
            ai.target_speed = v;
        }
        true
    }

    /// F-90720: the patrol path whose points' mean squared distance to the player is least (`free`: only the paths no
    /// other unit has); with `relocate`, a unit the player cannot see is moved to a hidden point of it - from a random
    /// start, the first within `vis` of the player, else the first hidden one, else none - and AssignPath (F-55649)
    /// starts its route there (+912; up there if underground). 0 done, 2 no path, 4 the path is taken.
    #[allow(clippy::too_many_arguments)]
    fn assign(&mut self, gpu: &Gpu, world: &World, vehicles: &mut Vehicles, ui: usize, free: bool, relocate: bool, vis: f32) -> u8 {
        let Some(pp) = vehicles.player().map(|c| c.state.pos) else { return 2 };
        let taken = |g: usize, units: &[Unit]| units.iter().enumerate().any(|(j, u)| j != ui && u.path == Some(g));
        let mean = |p: &Vec<Vec3>| p.iter().map(|q| q.distance_squared(pp)).sum::<f32>() / p.len().max(1) as f32;
        let best = self.paths.iter().enumerate().filter(|(g, p)| !p.is_empty() && !(free && taken(*g, &self.units))).min_by(|a, b| mean(a.1).total_cmp(&mean(b.1))).map(|(g, _)| g);
        let Some(g) = best else { return 2 };
        if taken(g, &self.units) {
            return 4;
        }
        let car = Self::unit_car(vehicles, self.units[ui].id);
        let upos = car.map_or(self.units[ui].slot, |i| vehicles.cars[i].state.pos);
        let blocked = |a: Vec3, b: Vec3| {
            let (a, b) = (a + Vec3::Y * 2.0, b + Vec3::Y * 2.0);
            let d = a.distance(b);
            d > 1e-4 && world.collider.raycast(a, (b - a) / d, d).is_some()
        };
        let path = self.paths[g].clone();
        let mut start = None;
        if relocate && (car.is_none() || blocked(upos, pp)) {
            let s0 = (self.rand() * path.len() as f32) as usize % path.len();
            let order: Vec<usize> = (0..path.len()).map(|k| (s0 + k) % path.len()).collect();
            start = order.iter().copied().find(|&k| path[k].distance(pp) < vis && blocked(path[k], pp)).or_else(|| order.iter().copied().find(|&k| blocked(path[k], pp)));
        }
        let k = start.unwrap_or_else(|| nearest_point(&path, upos));
        let u = &mut self.units[ui];
        u.path = Some(g);
        u.pi = (k + 1) % path.len();
        u.at = path[k];
        let fwd = (path[(k + 1) % path.len()] - path[k]).normalize_or(Vec3::X);
        // (relocated: put at the point, then F-19049 - onto the nearest rail, radii 5 .. 80 m with the ground probe, along
        // it at its speed; the path's point itself with no rail found)
        let railed = start.and_then(|_| crate::roamers::rail_place(world, path[k]));
        let (at, dir, v) = railed.map_or((path[k] + Vec3::Y * 0.8, fwd, 0.0), |(q, _, f, s)| (q, f, s));
        match car {
            None => {
                let Some(uid) = self.unit_surface(gpu, world, vehicles, ui, at, dir) else {
                    // (the port's: no car where the world is not streamed in yet - tried again next frame)
                    self.units[ui].path = None;
                    return 3;
                };
                if let Some(c) = vehicles.cars.iter_mut().find(|c| c.uid == uid) {
                    c.state.vel = dir * v;
                }
            }
            Some(i) if start.is_some() => {
                let c = &mut vehicles.cars[i];
                let q = glam::Quat::from_rotation_y((-dir.x).atan2(-dir.z));
                if let Some(vt) = c.params().veh.clone() {
                    crate::physics::vehsim::set_pose(&mut c.state, &vt, &world.collider, [q * Vec3::X, q * Vec3::Y, q * Vec3::Z, at], dir * v, Vec3::ZERO);
                }
                c.pose_history.reset();
                if let Some(ai) = c.ai.as_mut() {
                    ai.path = None;
                    ai.link = None;
                }
            }
            Some(_) => {}
        }
        let rail = railed.map_or(String::new(), |(q, _, _, s)| format!(", on the rail {:.1} m from the point at {:.1} m/s", q.distance(path[k]), s));
        log::info!("police: unit {} on patrol path {} from point {} ({:.0} m from the player{}{})", self.units[ui].id, g, k, path[k].distance(pp), if start.is_some() { ", moved out of sight" } else { "" }, rail);
        0
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
        self.reset_arrest();
        self.spawn_timer = 8.0;
        vehicles.cars.retain(|c| !c.cop);
    }

    /// Keep the arrest going for this long (its cutscene is playing).
    pub fn hold_busted(&mut self, secs: f32) {
        if self.heat == Heat::Busted {
            self.busted_timer = self.busted_timer.max(secs);
        }
    }

    /// No race's zone: none running, or the next race picks its own (a series goes from race to race without `race == None`).
    pub fn zone_clear(&mut self) {
        self.zone = None;
        self.zone_go = false;
        self.zone_target = None;
        self.racer_uids.clear();
        self.zone_picked = false;
    }

    /// The launching script's EvaluatePolice(race, chance, chase type, racer, a, b, c) (the same function in all 41
    /// launchers, e.g. TournamentCore 0x99e; docs/reference/hookmen/evaluate_police.md): a chance above 0 becomes 1 after
    /// four no-cop races in a row; a roll RAND_FLOAT_RANGE(0, 1) above the chance, or a race without cop zones, destroys
    /// every police car (DestroyAllPolice) and counts a no-cop race when the chance was above 0; else the script's three
    /// CreatePolice (units 400 - 402) and the streak back to 0. Then SetChaseProperties(a, b, c), SetRaceChaseTarget.
    /// `racers` is the race's racer count (+3080, the player included) for the launchers' gate: more than
    /// mcRacerManager_GetNumRacersAllowedForPolice (5) - chance 0. The roll is RAND_FLOAT_RANGE (F-47737) on the
    /// launching script's generator (`script_rng`).
    pub fn evaluate(&mut self, e: PoliceEval, racers: usize, zones: usize, unlocked: bool, vehicles: &mut Vehicles) {
        let mut chance = if e.gate5 && racers > 5 { 0.0 } else { e.chance };
        if chance > 0.0 && self.no_cop_races >= 4 {
            chance = 1.0;
        }
        let r = self.script_rng.get_or_insert_with(Mwc::script).float_range(0.0, 1.0);
        if zones == 0 || r > chance {
            self.destroy_all(vehicles);
            if chance > 0.0 {
                self.no_cop_races += 1;
            }
            self.race_off = true;
        } else {
            self.create_units(unlocked);
            self.no_cop_races = 0;
            self.race_off = false;
        }
        log::info!("police: EvaluatePolice chance {} (roll {:.3}, {} zones) - {}; {} no-cop races in a row", chance, r, zones,
                   if self.race_off { "no police in this race" } else { "police" }, self.no_cop_races);
        self.set_chase_properties(e.props);
        self.chase_type = e.chase_type;
        self.chase_player = e.chase_player;
    }

    /// PoliceManager_SetChaseProperties (F-95292 -> F-39756): the running tactic gets +29 = a with its track time
    /// F-79519(a ? (+136 > 0 ? +136 : 1) : -1), +30 = b, +31 = c; the manager keeps them for the next (+220..+222).
    pub fn set_chase_properties(&mut self, p: [bool; 3]) {
        if self.kind != TacKind::None {
            self.track_t = if p[0] { if self.track_t > 0.0 { self.track_t } else { 1.0 } } else { -1.0 };
            self.lost_ok = false;
            self.tac_retarget = p[1];
        }
        self.chase_props = p;
    }

    /// The free roam after a race: SetChaseProperties(1, 1, 1) (BEAT_SH_04B after its race, 0x35ff; the other launchers
    /// never change them) and the zone's chase target back to the roll.
    pub fn race_left(&mut self) {
        if self.chase_props != [true; 3] {
            self.set_chase_properties([true; 3]);
        }
        self.chase_type = 0;
        self.chase_player = false;
    }

    /// PoliceManager_EvaluateCopZones (F-88125): of the race's zones ONE is picked, and ONE lap
    /// (`rand % laps` when the race has more than one, else lap 0); lap 0's zone is armed at once,
    /// another's when the race's lap changes to it (F-82127, "[COP ZONE] lap [%d], cop zone enabled").
    /// (approx.: the port's own random numbers, not the game's LCG)
    pub fn pick_zone(&mut self, zones: usize, laps: usize) {
        self.zone_go = false;
        // (the manager's police list, F-53190: the units CreatePolice made; the dev override F-47443 is never set)
        let police = self.units.len() as i32;
        let picked = zone_pick(&mut self.rdo_rng, None, police, zones.min(u16::MAX as usize) as u16, Some(laps as i32));
        self.zone = picked.map(|(idx, lap)| {
            log::info!("police: cop zone {} enabled on lap {}", idx, lap);
            CopZone { idx: idx as usize, lap: lap as usize, state: if lap == 0 { 1 } else { 0 }, timer: 0.0, target: None, player: false, stationed: false }
        });
        if picked.is_none() {
            log::info!("police: no cops in this race");
        }
    }

    /// The zone machine of a race (F-71212), per frame: `lap` is the race's current lap, `racers` the race's racers in
    /// order (uid, the player, position, damaged out), `zone` the zone's place and radius.
    /// State 1 (armed): the first racer within the radius sets it off (`zone_set_off`) - state 2, 5.5 s, and the target
    /// (+200): BEAT_SH_04B's scripted racer, the player; else by the type (+204, SetRaceChaseTarget): 0 the roll (one draw
    /// of the shared generator against iRaceTgtAiProbPrcnt %), then the AI racer nearest to the racer within
    /// fRaceTgtAiDist (or that negative), else the racer itself; 1 the racer; 2 the nearest without the roll. The AI list
    /// is the race's AI racers in race order (the original's opponent manager's). State 2: the target damaged out
    /// (F-12032) - the chase starts at once (state 0); else the 5.5 s down, then state 3 for fCopZoneRdoDelayTime;
    /// state 3: the time down, and at < 0 or the target damaged out the chase starts: the Arrow Trap (type 4, StartTactic
    /// F-33550) on the target, player or AI racer.
    pub fn zone_step(&mut self, lap: usize, racers: &[(u32, bool, Vec3, bool)], zone: (Vec3, f32), dt: f32) -> Option<&'static str> {
        // (test hook MCLA_COP_ZONE_AI=1: the roll always picks the nearest AI racer, at any distance)
        let prob = if std::env::var_os("MCLA_COP_ZONE_AI").is_some() { 100 } else { self.tun.get("iRaceTgtAiProbPrcnt", 30.0) as i32 };
        let ai_r = if std::env::var_os("MCLA_COP_ZONE_AI").is_some() { -1.0 } else { self.tun.get("fRaceTgtAiDist", 150.0) };
        let delay = self.tun.get("fCopZoneRdoDelayTime", 2.0);
        let (kind, scripted) = (self.chase_type as u32, if self.chase_player { racers.iter().position(|r| r.1) } else { None });
        let Some(z) = self.zone.as_mut() else { return None };
        let mut say = None;
        let wrecked = |z: &CopZone| z.target.and_then(|t| racers.iter().find(|r| r.0 == t)).map_or(false, |r| r.3);
        let mut start = false;
        match z.state {
            0 if lap == z.lap => {
                z.state = 1;
                log::info!("police: lap {}, cop zone enabled", lap);
            }
            // (F-71212: no racer sets it off while the player drives a police car)
            1 if self.player_police => {}
            1 => {
                let pos: Vec<Vec3> = racers.iter().map(|r| r.2).collect();
                let ai: Vec<usize> = (0..racers.len()).filter(|&i| !racers[i].1).collect();
                let ai_pos: Vec<Option<Vec3>> = ai.iter().map(|&i| Some(racers[i].2)).collect();
                if let Some((_, tgt)) = zone_set_off(&mut self.rdo_rng, kind, prob, ai_r, zone.0, zone.1, &pos, scripted, Some(&ai_pos)) {
                    z.state = 2;
                    z.timer = 5.5;
                    say = Some("PLAYER_COPS_BEGIN_PURSUIT");
                    let t = tgt.map(|t| match t {
                        ZoneTgt::Racer(i) => i,
                        ZoneTgt::Ai(j) => ai[j],
                    });
                    z.target = t.map(|i| racers[i].0);
                    z.player = t.map_or(false, |i| racers[i].1);
                    log::info!("police: cop zone {} set off, target {}", z.idx,
                               if t.is_none() { "none" } else if z.player { "the player" } else { "an AI racer" });
                }
            }
            2 | 3 => {
                let w = wrecked(z);
                (z.state, z.timer, start) = zone_tick(z.state, z.target.is_some(), z.timer, dt, delay, || w);
            }
            _ => {}
        }
        if start {
            z.state = 0;
            z.lap = usize::MAX;
            if z.player && matches!(self.heat, Heat::None | Heat::PullOver) {
                self.zone_go = true;
            } else if !z.player {
                // (the Arrow Trap on the AI racer: started by the next update, `update`)
                self.zone_target = z.target;
            }
        }
        say
    }

    /// PoliceManager_IsRacerPursued's zone clause (F-86429): the zone's state +196 is 2 or 3
    /// and its target +200 is the player.
    pub fn zone_on_player(&self) -> bool {
        self.zone.as_ref().map_or(false, |z| matches!(z.state, 2 | 3) && z.player)
    }

    /// Test helper: start a pursuit right now with a cruiser close by.
    /// Test hook MCLA_COP_TEST=units: a zone's chase with no test cruiser - the engage takes the units.
    pub fn debug_units(&mut self) {
        self.score = 700.0;
        self.zone_go = true;
    }

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
        (self.flee_t, self.flee_ask) = (0.0, false);
        self.last_known = at;
        self.fine = self.law_fines();
        self.lost = 0.0;
        self.reset_arrest();
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
        // (F-11097: the candidates are every unit by its squared distance from where it is now - an underground one sorts
        // by its slot; the port's cars are taken first above, then the underground units by their slots)
        let _ = fwd;
        let mut under: Vec<(f32, usize)> = self.units.iter().enumerate().filter(|(_, u)| u.under && Self::unit_car(vehicles, u.id).is_none()).map(|(i, u)| (u.slot.distance_squared(at), i)).collect();
        under.sort_by(|a, b| a.0.total_cmp(&b.0));
        let all_need = out.is_empty();
        let mut junctions = true;
        for (d2, ui) in under {
            if out.len() >= n || goons {
                break;
            }
            // (the tactic's teleport, F-37056, brings it up out of the target's sight; the port does it at the engage -
            // a unit that finds no place leaves the tactic to be tried again next frame)
            let Some(p) = self.target_car(vehicles) else { break };
            let dims = vehicles.models.iter().find(|m| m.name == COP_CAR).map_or((2.5, 1.0, 1.5), |m| (0.5 * (m.max.z - m.min.z), 0.5 * (m.max.x - m.min.x), m.max.y - m.min.y));
            let tgt = (p.state.pos, p.state.vel, p.state.forward(), p.model.max.y - p.model.min.y);
            let Some((q, row2, vel)) = self.place_cop(world, tgt, d2, all_need, &mut junctions, dims, self.fw_road && self.offences & LAW_WRONG_WAY == 0) else { return None };
            let Some(uid) = self.unit_surface(gpu, world, vehicles, ui, q, -row2) else { return None };
            if let Some(c) = vehicles.cars.iter_mut().find(|c| c.uid == uid) {
                c.state.vel = vel;
            }
            log::info!("police: unit {} up for the tactic at {:?} ({:.0} m from the target)", self.units[ui].id, q, q.distance(at));
            out.push(uid);
        }
        // (F-11097 calls F-97878 on each cop it takes: +1072 (may check law breaking, no port field) = +1073 = 1 - an
        // engaged unit may patrol again after the tactic, whatever SetAllPoliceActive / a failed assign left)
        for u in self.units.iter_mut() {
            if Self::unit_car(vehicles, u.id).is_some_and(|ci| out.contains(&vehicles.cars[ci].uid)) {
                u.may_patrol = true;
                u.may_check = true;
            }
        }
        (out.len() >= n).then_some(out)
    }

    /// The tactic teleport's place (F-37056): the road under the target (3 m, else 125 m), its junction behind (every cop
    /// needing it) or ahead, then the road itself; each by its lane's 8 points (F-13354), else up to 3 connection-list
    /// hops away (F-92039); no road: a 45 degree ring 125 m ahead. A point is closer to the target than the cop is now
    /// (`cop_d2`) and its 4 footprint corners are hidden from the target's roof. Out: the place (+0.25 m), row 2 (the
    /// car's forward is -row 2) and the velocity: 80 mph forward when the target moves along row 2, else standing.
    #[allow(clippy::too_many_arguments)]
    fn place_cop(&mut self, world: &World, tgt: (Vec3, Vec3, Vec3, f32), cop_d2: f32, all_need: bool, junctions: &mut bool, dims: (f32, f32, f32), fw: bool) -> Option<(Vec3, Vec3, Vec3)> {
        let nav = world.nav.as_ref()?;
        let (tpos, tvel, tfwd, th) = tgt;
        let (hl, hw, ch) = dims;
        let eye = tpos + Vec3::Y * th;
        let hidden = |q: Vec3| {
            let d = q.distance(eye);
            d > 1e-4 && world.collider.raycast(eye, (q - eye) / d, d).is_some()
        };
        let ok = |q: Vec3, dir: Vec3| {
            let dir = Vec3::new(dir.x, 0.0, dir.z).normalize_or(Vec3::Z);
            let side = Vec3::new(-dir.z, 0.0, dir.x);
            // (FW: only points not ahead of the target, dot(target row2, q - target) >= 0)
            (!fw || (-tfwd).dot(q - tpos) >= 0.0) && q.distance_squared(tpos) < cop_d2 && [(1.0, 1.0), (1.0, -1.0), (-1.0, 1.0), (-1.0, -1.0)].iter().all(|&(a, b)| hidden(q + dir * hl * a + side * hw * b))
        };
        let try_node = |n: NavNode| -> Option<(Vec3, Vec3)> {
            let curve = nav.curves.get(nav.node_rect(n).lane?)?;
            let near0 = curve.eval(0.0).0.distance_squared(tpos) <= curve.eval(1.0).0.distance_squared(tpos);
            (0..8).map(|k| if near0 { k } else { 7 - k }).find_map(|k| {
                let (p, d) = curve.eval(k as f32 / 8.0);
                let q = p + Vec3::Y * ch;
                ok(q, d).then_some((q, d.normalize_or(Vec3::Z)))
            })
        };
        let road = nav.node_within(tpos, 3.0, false).or_else(|| nav.node_within(tpos, 3.0, true)).or_else(|| nav.node_within(tpos, 125.0, false)).or_else(|| nav.node_within(tpos, 125.0, true));
        let found = match road {
            Some(r) => {
                let heading = if tvel.length() >= 2.235 { tvel } else { tfwd };
                let mut order = Vec::new();
                if let (NavNode::Road(_), true) = (r, *junctions) {
                    let ends: Vec<NavNode> = nav.next_of(r).iter().copied().filter(|x| matches!(x, NavNode::Junction(_))).take(2).collect();
                    let ahead = |j: &NavNode| {
                        let c = nav.node_rect(*j).c;
                        (c[0] - tpos.x) * heading.x + (c[1] - tpos.z) * heading.z > 0.0
                    };
                    let (a, b): (Vec<NavNode>, Vec<NavNode>) = ends.into_iter().partition(ahead);
                    // (FW / XT records - the freeway's - never take the junction ahead)
                    match (b.first(), a.first()) {
                        (Some(&jb), _) if all_need => order.push(jb),
                        (_, Some(&ja)) if !fw => order.push(ja),
                        _ => {}
                    }
                    *junctions = false;
                }
                order.push(r);
                let mut hit = None;
                for n in order {
                    if let Some(h) = try_node(n) {
                        hit = Some((h, None));
                        break;
                    }
                    // grow(node, 3): the connection list, its order reversed on every other call, try each, then each's
                    let mut level = vec![n];
                    'grow: for _ in 0..3 {
                        let mut next = Vec::new();
                        for &m in &level {
                            let mut list: Vec<NavNode> = nav.next_of(m).to_vec();
                            if !self.grow_flip {
                                list.reverse();
                            }
                            self.grow_flip = !self.grow_flip;
                            for &x in &list {
                                if let Some(h) = try_node(x) {
                                    hit = Some((h, Some(x)));
                                    break 'grow;
                                }
                            }
                            next.extend(list);
                        }
                        level = next;
                    }
                    if hit.is_some() {
                        break;
                    }
                }
                hit.map(|((q, mut d), via)| {
                    // (found through the growth: turned so the car's forward points at the node of the connection list
                    // nearest the target)
                    if let Some(x) = via {
                        if let Some(c) = nav.next_of(x).iter().map(|&y| nav.node_rect(y).c).min_by(|a, b| ((a[0] - tpos.x).powi(2) + (a[1] - tpos.z).powi(2)).total_cmp(&((b[0] - tpos.x).powi(2) + (b[1] - tpos.z).powi(2)))) {
                            if (-d).dot(Vec3::new(c[0] - q.x, 0.0, c[1] - q.z)) < 0.0 {
                                d = -d;
                            }
                        }
                    }
                    (q, d)
                })
            }
            None => {
                // (no road within 125 m: 125 m ahead turned by a random eighth, 8 tries a 45 degree turn each; over the mesh,
                // closer than the cop, hidden, and ground within 7 m)
                let mesh = crate::offroad::mesh();
                let k0 = (self.rand() * 8.0) as i32;
                (0..8).find_map(|k| {
                    let off = glam::Quat::from_rotation_y((k0 + k) as f32 * std::f32::consts::FRAC_PI_4) * (tfwd * 125.0);
                    let q = tpos + off;
                    if !mesh.map_or(true, |m| m.on_mesh(q)) || !ok(q, -tfwd) {
                        return None;
                    }
                    world.collider.raycast(q + Vec3::Y * 7.0, -Vec3::Y, 14.0).map(|h| (h.point, -tfwd))
                })
            }
        };
        let (q, row2) = found?;
        let q = q + Vec3::Y * 0.25;
        let row2 = Vec3::new(row2.x, 0.0, row2.z).normalize_or(Vec3::Z);
        let vel = if tvel.dot(row2) > 0.0 { -row2 * 35.76 } else { Vec3::ZERO };
        Some((q, row2, vel))
    }

    /// The tactic base update's first step (F-63286 -> F-24596 -> F-37056): when the gate allows, every tactic cop not
    /// seen by the target that "needs" it (more than 5 s off the nav mesh) or - healthy - is beyond fTeleportDist, with
    /// probability iTeleportProbPrcnt x d / fTeleportDist and fTeleportDownTime of cooldown, is put at a hidden place.
    fn teleport(&mut self, world: &World, vehicles: &mut Vehicles, tun: &Tuning, dt: f32) {
        self.tac_t += dt;
        self.tel_lock = (self.tel_lock - dt).max(0.0);
        let mesh = crate::offroad::mesh();
        for (uid, e) in self.tel.iter_mut() {
            e.0 = (e.0 - dt).max(0.0);
            if let Some(c) = vehicles.cars.iter().find(|c| c.uid == *uid) {
                e.1 = if mesh.map_or(true, |m| m.on_mesh(c.state.pos)) { 0.0 } else { e.1 + dt };
                // (the car's mcCarDamage: damaged out (+36 >= MaxDamage), then its DamageOutDelay countdown +44; -1 = not
                // damaged out)
                e.2 = if !damaged_out(c.state.damage) {
                    -1.0
                } else if e.2 < 0.0 {
                    c.model.simtunes.as_ref().map_or(0.0, |s| s.base.damage.damage_out_delay)
                } else {
                    (e.2 - dt).max(0.0)
                };
            }
        }
        for u in &self.cops {
            self.tel.entry(*u).or_insert((0.0, 0.0, -1.0));
        }
        let cops = self.cops.clone();
        // (F-14059: underground - no car here -, over 5 s off the nav mesh (brain +584), or damaged out with its
        // damage-out delay at or under 0 (car +20 -> +36 >= MaxDamage and +44 <= 0))
        // (the port's units underground have no car and so no entry here; e.2 >= 0 is the damaged-out countdown)
        let needs = |s: &Self, u: u32| s.tel.get(&u).is_some_and(|e| needs_teleport(false, e.1, e.2 >= 0.0, e.2));
        let any_need = cops.iter().any(|&u| needs(self, u));
        let no_tel = tun.get("fNoTeleportTime", 60.0);
        let gate = self.tel_lock <= 0.0
            && (any_need || (!self.in_race && cops.len() >= 2 && self.heat != Heat::Search && !(no_tel > 0.0 && self.tac_t >= no_tel)));
        if !gate || self.kind <= TacKind::Dispatch {
            return;
        }
        let Some(p) = self.target_car(vehicles) else { return };
        let tgt = (p.state.pos, p.state.vel, p.state.forward(), p.model.max.y - p.model.min.y);
        let all_need = cops.iter().all(|&u| needs(self, u));
        let far = tun.get("fTeleportDist", 250.0);
        let prob = (tun.get("iTeleportProbPrcnt", 75.0) * 0.01).clamp(0.0, 1.0);
        let down = tun.get("fTeleportDownTime", 8.0);
        let mut junctions = true;
        for &u in &cops {
            let Some(i) = vehicles.cars.iter().position(|c| c.uid == u) else { continue };
            if self.tel.get(&u).is_some_and(|e| e.0 > 0.0) {
                continue;
            }
            let c = &vehicles.cars[i];
            let ch = c.model.max.y - c.model.min.y;
            let (eye, top) = (tgt.0 + Vec3::Y * tgt.3, c.state.pos + Vec3::Y * ch);
            let dd = eye.distance(top);
            if dd < 1e-4 || world.collider.raycast(eye, (top - eye) / dd, dd).is_none() {
                continue;
            }
            let d = c.state.pos.distance(tgt.0);
            let need = needs(self, u);
            let mut p_tel = 1.0;
            if !need {
                if d <= far {
                    continue;
                }
                p_tel = if far > 0.0 { prob * (d / far) } else { prob };
                self.tel.entry(u).or_default().0 = down;
                self.tel_lock = down;
                if self.rand() >= p_tel {
                    continue;
                }
            }
            let dims = (0.5 * (c.model.max.z - c.model.min.z), 0.5 * (c.model.max.x - c.model.min.x), ch);
            match self.place_cop(world, tgt, d * d, all_need, &mut junctions, dims, self.fw_road && self.offences & LAW_WRONG_WAY == 0) {
                Some((q, row2, vel)) => {
                    let c = &mut vehicles.cars[i];
                    let f = -row2;
                    let rot = glam::Quat::from_rotation_y((-f.x).atan2(-f.z));
                    if let Some(vt) = c.params().veh.clone() {
                        crate::physics::vehsim::set_pose(&mut c.state, &vt, &world.collider, [rot * Vec3::X, rot * Vec3::Y, rot * Vec3::Z, q], vel, Vec3::ZERO);
                    }
                    c.pose_history.reset();
                    if let Some(ai) = c.ai.as_mut() {
                        ai.path = None;
                        ai.link = None;
                        if let Some(b) = ai.brain.0.as_mut() {
                            b.set_route(&[tgt.0], -1.0, Vec3::ZERO);
                        }
                    }
                    if let Some(e) = self.tel.get_mut(&u) {
                        e.1 = 0.0;
                    }
                    log::info!("police: cruiser {} teleported to {:?} ({:.0} m from the target, was {:.0}{})", u, q, q.distance(tgt.0), d, if need { ", off the mesh" } else { "" });
                }
                None if p_tel > 0.99 => {
                    self.tel.entry(u).or_default().0 = 0.0;
                }
                None => {}
            }
        }
    }

    /// The dispatch cop's search (cmd 7, +736 = 1: from its own place, a random turn at each junction, the first
    /// +-pi/2 at random; F-52732 / F-92001). The direction: the target's velocity now (the brain's last known
    /// velocity of a dispatch was not read).
    fn dispatch_search(&mut self, world: &World, vehicles: &Vehicles, uid: u32, per: f32, brain: &mut BrainCmd) {
        let (Some(c), Some(nav)) = (vehicles.cars.iter().find(|c| c.uid == uid), world.nav.as_ref()) else { return };
        // (the search reads the brain's +624, never written during the dispatch - command 8 - so it is what this cop last
        // stored, zero for one whose first command is the dispatch: then (1, 0, 0) x 22.35 in search_points)
        let v = self.brain_tvel.get(&uid).copied().unwrap_or(Vec3::ZERO);
        self.last_known_v = v;
        let angle = if crate::brain::rng().float() < 0.5 { std::f32::consts::FRAC_PI_2 } else { -std::f32::consts::FRAC_PI_2 };
        let mut rnd = || crate::brain::rng().float();
        let pts = search_points(nav, world, c.state.pos, v, per, true, true, angle, &mut rnd);
        log::info!("police: dispatch cruiser {} searches: {} goals", uid, pts.len());
        self.srch.retain(|s| s.uid != uid);
        self.srch.push(search_start(uid, pts, true, angle));
        // (command 7 with +16 = 1 and the angle, F-60530's block, through the brain command F-83687)
        brain_command(brain, 7, Some(&search_block(true, angle)), 0);
    }

    /// The tactic's target car (+32): the AI racer the cop zone chose, else the player.
    fn target_car<'a>(&self, vehicles: &'a Vehicles) -> Option<&'a crate::vehicles::CarInstance> {
        match self.target {
            Some(u) => vehicles.cars.iter().find(|c| c.uid == u),
            None => vehicles.player(),
        }
    }

    /// StartTactic (F-33550): the old tactic ends with flag 0 (its cops lose their commands; lights kept), the score
    /// goes to the kind's floor; the base begin (F-82656): +136 = fTrackTgtTime (the manager's track byte, 1 by default -
    /// F-39756) for the kinds whose slot 7 reads it, -1 (never lost) for the pull-over.
    fn start_tactic(&mut self, kind: TacKind, cops: Vec<u32>, may_escalate: bool, track: f32) {
        log::info!("police: tactic {:?} -> {:?} with cruisers {:?}", self.kind, kind, cops);
        // (the interrupt comes before the same-kind test: every start of a kind other than Dispatch (type 0))
        self.phone_cut |= !matches!(kind, TacKind::None | TacKind::Dispatch);
        let old = (self.kind != TacKind::None).then_some((self.kind.ty(), self.may_escalate));
        // (the tutorial F-24639: kind >= 2 on the player's car - the target's vt+40 - when the kind is 2 or no old tactic ended;
        // every caller passes the show flag r6 = 1)
        self.pursuit_tut |= self.target.is_none() && kind.ty() >= 2 && (kind.ty() == 2 || old.is_none());
        // (the strip's texts, F-76838: a Dispatch 2; a Pull-over 0 then 3; the others 1 then 3 - on a start only)
        if kind != TacKind::None && matches!(start_rule(kind.ty(), old), StartRule::Start { .. }) {
            self.hud_msgs.extend_from_slice(match kind {
                TacKind::Dispatch => &[2][..],
                TacKind::PullOver => &[0, 3],
                _ => &[1, 3],
            });
        }
        // (the stat gCPA + 1 for a kind >= 1 with no old tactic or an old Dispatch, any target; the event POLICE_PURSUIT
        // for a kind >= 2 (F-41015) has no listener - no file of the disc and no other code of the image names it)
        if kind.ty() >= 1 && old.map_or(true, |(t, _)| t == 0) {
            self.stat_cpa += 1;
        }
        // (the manager's +84 on a pull-over start only: the tactic's first cop)
        if kind == TacKind::PullOver {
            self.last_pullover = cops.first().copied();
        }
        self.kind = kind;
        self.cops = cops;
        self.probe_cop = None;
        self.may_escalate = may_escalate;
        // (the tactic's begin: +112 = fMinPerRdoTime; its constructor: +96 = 1, +100 = 0)
        self.rdo_t = self.tun.get("fMinPerRdoTime", 15.0);
        self.rdo_nitro = 0;
        self.rdo_far = true;
        // (F-63286: the duration +104 from 0, unless StartTactic carries the old tactic's - F-33550, `start_rule`; +120 = the
        // request's +16 - 0 here)
        if !matches!(start_rule(kind.ty(), old), StartRule::Start { carry: true, .. }) {
            self.tac_t = 0.0;
        }
        self.tel_lock = 0.0;
        self.tel.clear();
        self.tacs.clear();
        self.srch.clear();
        self.eyes.clear();
        self.sq = Squad { hyst: 2.0, lock: 0.0, ..Default::default() };
        // (the floor only with a law record, the player's: F-33550)
        if self.target.is_none() {
            self.score = self.score.max(kind.floor());
        }
        // (the base begin: F-79519(+29 ? fTrackTgtTime : -1); +29 / +30 from the manager's +220 / +221 via the request)
        self.track_t = if kind == TacKind::PullOver || !self.chase_props[0] { -1.0 } else { track };
        self.lost_ok = false;
        self.tac_retarget = self.chase_props[1];
        if kind != TacKind::Dispatch {
            self.dsp = None;
        }
    }

    /// PoliceManager_CheckPoliceActivity(1) (F-35540) as far as the port's states reach: -1 a dispatch (group 0) to
    /// the player past its wait (state > 0, tested first), 2 a pursuit, its search or the arrest, 1 a pull-over, else 0.
    /// With -1 the dispatch's state (F-48923 reads it: 2 = Respond).
    pub fn activity(&self) -> (i32, u8) {
        if let Some(d) = self.dsp.as_ref().filter(|d| d.st > 0) {
            return (-1, d.st);
        }
        (match self.heat {
            Heat::None => 0,
            Heat::PullOver => 1,
            Heat::Pursuit | Heat::Search | Heat::Busted => 2,
        }, 0)
    }

    /// PoliceManager_CheckPoliceActivity(1) (F-35540) in full for the strip's look: -1 a Dispatch (type 0) on the player past
    /// its wait (tested first); then over the tactics on the player, 3 one in its Search with the clock running (vt+44, the
    /// target lost - F-63053), 2 one in its Search (vt+40, F-77780); then the tactic of type 4 / 3 / 2 / 1 (Arrow Trap,
    /// Double Pursuit, Pursuit, Pull-over: 7 / 6 / 5 / 4 - F-10826); else 0. (The manager's +56 byte -> 100, the +196 test
    /// -> 7 and the units' test -> 1 give the strip's look 0 or 1 as their neighbours; the +196 and units' tests are not
    /// reached by the port.) +56 is the arrest's hold: F-22530 sets it on the success frame (on both of its arrest paths, before
    /// COP_ARREST) and CutSceneDone clears it - the port's posted arrest, held as `Heat::Busted` through the scene.
    pub fn hud_state(&self) -> i32 {
        if self.dsp.as_ref().is_some_and(|d| d.st > 0) {
            return -1;
        }
        if self.target.is_some() {
            return 0;
        }
        if self.heat == Heat::Search {
            return if search_clock_runs(self.search) { 3 } else { 2 };
        }
        // (after the Search tests, before the tactics' types, as F-35540 tests it)
        if self.heat == Heat::Busted && self.arrest_posted {
            return 100;
        }
        match self.kind {
            TacKind::ArrowTrap => 7,
            TacKind::PursuitDbl => 6,
            TacKind::Pursuit => 5,
            TacKind::PullOver => 4,
            _ => 0,
        }
    }

    /// The tactic's radio, once a game frame (F-26639, from the tactic's update): the countdown always; a line only in a
    /// Pursuit past its Delay, a Double Pursuit or an Arrow Trap, not in the Search, and with no line playing. The rule is
    /// `chatter_step`; this gathers its inputs from the world.
    pub fn chatter(&mut self, world: &World, vehicles: &Vehicles, busy: bool, dt: f32) -> Option<RadioCall> {
        let on = match self.kind {
            TacKind::Pursuit => self.flee_hold <= 0.0,
            TacKind::PursuitDbl | TacKind::ArrowTrap => true,
            _ => false,
        };
        let car = self.target_car(vehicles);
        let cops = self.cops.clone();
        let n = cops.len();
        // (A and B read the target's law record: none for an AI racer - F-26639 skips them; C and D run on its car)
        let record = self.target.is_none();
        let (ppos, speed, nitro) = car.map_or((Vec3::ZERO, 0.0, false), |p| {
            (p.state.pos, p.state.speed(), p.state.veh.as_deref().is_some_and(|v| v.nitro_burning))
        });
        let nav = world.nav.as_ref().filter(|_| record);
        let is_fw = |r: usize| nav.and_then(|nv| nv.zones.get(r)).is_some_and(|z| z.name.starts_with("FW") || z.name.starts_with("XT"));
        // the road of the target's lane (the original skips A when it has none)
        let road = car
            .filter(|_| record)
            .and_then(|p| vehicles.driver_lanes.get(&p.uid))
            .and_then(|d| d.best)
            .and_then(|b| nav.and_then(|nv| nv.lane_road(b.0)));
        // the target's contact list, a cop's entry (F-13798): the hard-hit table the law block kept this frame
        let hit: Vec<Option<f32>> =
            cops.iter().map(|u| self.hard_tab.as_ref().and_then(|t| t.iter().find(|e| e.0 == *u).map(|e| e.1))).collect();
        // F-29443: some cop within the distance of the target (only asked when n > 1)
        let within = |d: f32| cops.iter().filter_map(|u| vehicles.cars.iter().find(|c| c.uid == *u)).any(|c| c.state.pos.distance(ppos) < d);
        let vic = self.tun.get("fRdoVicinityDist", 100.0);
        let (within_vic, within_10) = if n > 1 && car.is_some() { (within(vic), within(10.0)) } else { (false, false) };
        let mut st = RdoState { t: self.rdo_t, road: self.rdo_road, fw_t: self.rdo_fw_t, nitro: self.rdo_nitro, far: self.rdo_far, rng: self.rdo_rng };
        let i = RdoIn {
            dt,
            on: on && self.heat != Heat::Search && car.is_some(),
            busy,
            n,
            target: true,
            is_player: record,
            rec: record,
            road,
            is_fw: &is_fw,
            hard: self.hard_tab.is_some(),
            hit: &hit,
            nitro_burning: nitro,
            speed,
            in_race: self.in_race,
            // (F-76898: the cops in the latest tactic)
            mgr80: n as i32,
            within_vic,
            within_10,
            min_t: self.tun.get("fMinPerRdoTime", 15.0),
            max_t: self.tun.get("fMaxPerRdoTime", 30.0),
        };
        let call = chatter_step(&mut st, &i);
        self.rdo_t = st.t;
        self.rdo_road = st.road;
        self.rdo_fw_t = st.fw_t;
        self.rdo_nitro = st.nitro;
        self.rdo_far = st.far;
        self.rdo_rng = st.rng;
        call.map(|(name, k, r)| RadioCall { name, sender: cops[k], receiver: r.map(|r| cops[r]) })
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
        self.reset_arrest();
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

    /// Advance the police simulation. Returns what happened this frame.
    pub fn update(&mut self, gpu: &Gpu, world: &World, vehicles: &mut Vehicles, dt: f32) -> Option<Event> {
        self.start_report = None;
        self.report_bits = None;
        self.tick_law = 0;
        // (F-52240 clears the word at the end of a scored tick and in the grace: one payment a frame at most)
        let law_word = vehicles.player_mut().and_then(|c| c.state.veh.as_deref_mut()).map_or(0, |v| std::mem::take(&mut v.agro.law_word));
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
            // (the player's own cruiser is not one of them: its lights follow its HEADLIGHTS button - main.rs)
            let off = std::mem::take(&mut self.lamps_off);
            for c in vehicles.cars.iter_mut().filter(|c| c.cop && !c.is_player) {
                if off.contains(&c.uid) {
                    c.siren = false;
                }
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
        // ---- an AI racer as the tactic's target (docs/reference/hookmen/cop_zone.md) -------------------------
        let mut pre_event = None;
        // (the zone's start for an AI racer, F-71212 -> F-30913 -> StartTactic F-33550: the Arrow Trap on that racer, no
        // request cop, no start report; it has no law record - F-50742 gives one only to the player - so no score floor)
        if let Some(u) = self.zone_target.take() {
            let at = vehicles.cars.iter().find(|c| c.uid == u).map(|c| (c.state.pos, c.state.forward(), c.state.vel));
            if let (true, Some((pos, fwd, vel))) = (self.heat == Heat::None && self.in_race && !goons, at) {
                let prev = self.target.replace(u);
                if let Some(cops) = self.engage(gpu, world, vehicles, TacKind::ArrowTrap.cops(), None, pos, fwd) {
                    let tt = self.tun.get("fTrackTgtTime", 25.0);
                    self.start_tactic(TacKind::ArrowTrap, cops, true, tt);
                    self.heat = Heat::Pursuit;
                    self.last_known = pos;
                    self.last_known_v = vel;
                    self.lost = 0.0;
                    log::info!("police: the cop zone's Arrow Trap on AI racer {}", u);
                } else {
                    self.target = prev;
                    log::info!("police: the cop zone's Arrow Trap on AI racer {} found no three cruisers", u);
                }
            }
        }
        if let Some(u) = self.target {
            let alive = vehicles.cars.iter().any(|c| c.uid == u);
            let wrecked = vehicles.cars.iter().any(|c| c.uid == u && damaged_out(c.state.damage));
            if alive && wrecked && self.heat == Heat::Search {
                // (F-22530: the tactic lost and its target damaged out (F-12032) - F-41497 ends it, silently)
                log::info!("police: AI racer {} lost and damaged out - its tactic ends", u);
                self.end_tactic(true);
            } else if !alive {
                // (F-22530: the target's +656 no longer 3 - the tactic ends, silently)
                log::info!("police: AI racer {} gone - its tactic ends", u);
                self.end_tactic(true);
            } else if !self.in_race {
                // (F-22530: no race current and the target not the player - the tactic ends at once; a tactic cop seeing
                // the player within fVisionDist, F-43276 (no cone), starts an Arrow Trap on the player with that cop
                // as its request cop. The trap's delay 5.0 has no counterpart in the port's trap)
                let vision = self.tun.get("fVisionDist", 500.0);
                let seer = vehicles.player().and_then(|p| {
                    self.cops.iter().filter_map(|k| vehicles.cars.iter().find(|c| c.uid == *k)).find(|c| cop_sees(world, c, p.state.pos, vision, 0.0)).map(|c| (c.uid, p.state.pos, p.state.forward(), p.state.vel))
                });
                log::info!("police: AI racer {} chased with no race - the tactic ends{}", u, if seer.is_some() { ", the player seen" } else { "" });
                self.end_tactic(true);
                if let Some((cop, pos, fwd, vel)) = seer {
                    if let Some(cops) = self.engage(gpu, world, vehicles, TacKind::ArrowTrap.cops(), Some(cop), pos, fwd) {
                        let tt = self.tun.get("fTrackTgtTime", 25.0);
                        self.start_tactic(TacKind::ArrowTrap, cops, true, tt);
                        self.heat = Heat::Pursuit;
                        self.last_known = pos;
                        self.last_known_v = vel;
                        self.fine = self.law_fines();
                        pre_event = Some(Event::Pursuit);
                    }
                }
            }
        }
        // (only the tactic's cops chase; F-76774 result 3: one of them gone - the police are lost)
        let gone = self.kind > TacKind::Dispatch && !goons && self.cops.iter().any(|u| !vehicles.cars.iter().any(|c| c.uid == *u));
        if self.kind > TacKind::Dispatch && !goons {
            let tun = std::mem::take(&mut self.tun);
            self.teleport(world, vehicles, &tun, dt);
            self.tun = tun;
        }
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
        // (the tactic's target, +32: the player, or the AI racer the cop zone chose. The law record and its offences are
        // the player's; the tactic - sight, search, arrest tests, the cops' goals - looks at the target)
        let ai_target = self.target.is_some();
        // (the player's lane and its kept phase: the PullOver brain's +640 / +624 - an AI racer target's lane is not kept)
        let lane_of_player = if ai_target { None } else { vehicles.driver_lanes.get(&p.uid).and_then(|d| Some((d.best?.0, d.best_t))) };
        let tc = self.target_car(vehicles).unwrap_or(p);
        let (tpos, tvel, tfwd, tright, tdamage) = (tc.state.pos, tc.state.vel, tc.state.forward(), tc.state.right(), tc.state.damage);
        let t_burning = burn_charged(tc.state.veh.as_ref().map(|v| v.burn));
        let t_bike_down = tc.model.bike && tc.state.wipeout > 0.0;
        let (t_uid, t_half, t_rev) = (tc.uid, 0.5 * (tc.model.max.y - tc.model.min.y), tc.state.gear < 0);
        let prev_heat = std::mem::replace(&mut self.prev_heat, self.heat);
        if self.heat == Heat::Pursuit && !matches!(prev_heat, Heat::Pursuit | Heat::Search | Heat::Busted) {
            self.last_known_v = tvel;
        }
        // (F-63286: +136 runs down while above 0; when it passes 0 the brains' lost flag +773 is set if the tactic's
        // vt+28 answers - the Pursuit classes' +29, the pull-over's 0)
        let answers = self.chase_props[0] && matches!(self.kind, TacKind::Pursuit | TacKind::PursuitDbl | TacKind::ArrowTrap);
        track_step(&mut self.track_t, &mut self.lost_ok, dt, answers);
        let track_flag = self.lost_ok;
        let pfwd = p.state.forward();
        let speed = pvel.length();
        let mph = speed * MPH;
        let damage = p.state.damage;
        let p_air = p.state.airborne_time;
        // (F-81388: the gyro's two-wheel state, +18, is not 0)
        let p_two = p.state.veh.as_ref().map_or(false, |v| v.two != 0);
        let tun = std::mem::take(&mut self.tun);
        let t = |k: &str, d: f32| tun.get(k, d);

        // ---- what the cruisers can see ---------------------------------------
        let detect = t("fDetectDist", 40.0);
        let vision = t("fVisionDist", 500.0);
        // (the half angle of F-24233 / F-87106's sight tests: the member, 180 on the disc = no cone in practice)
        let det_half = t("fDetectHalfAngle", 180.0);
        let mut seen = false;
        let mut n_cops = 0;
        // (F-94676 for a free cop - in no tactic, not the dispatch's -, F-52240: within fDetectDist in 3D and a clear
        // line from 2 m above the cop to 2 m above the target, F-69576; within fDetectHalfAngle of its forward, F-24233.
        // The nearest such cop is the request cop of the tactic it starts or joins.)
        let dsp_cop = self.dsp.as_ref().map(|d| d.cop);
        let mut detector: Option<(f32, u32)> = None;
        // (F-52240's loop takes a unit only with +872 and +1072 set: a unit not allowed to check law breaking detects nothing)
        // (F-52240 asks F-76033(need_check, need_free, group 1): the first unit in the manager's order, not the nearest. The
        // manager's order is the units' (created at the player's load, 400 - 402, and kept while underground); a cop car of
        // no unit (the race's waiting cruisers) comes after them in the cars' order - their place in the original's list is
        // not decoded. Group 1 = a real cop (the goons' +872 is 0); in a tactic = a pursuer or the dispatch's cop)
        let mut cands: Vec<(usize, &crate::vehicles::CarInstance)> = vehicles.cars.iter().filter(|c| c.cop).map(|c| {
            let ui = self.units.iter().position(|u| c.vinyl_name == format!("police_unit:{}", u.id));
            (ui.unwrap_or(self.units.len()), c)
        }).collect();
        cands.sort_by_key(|x| x.0);
        let pick = first_detecting(cands.len(), true, true, 1, |i| {
            let (ui, c) = cands[i];
            Some((self.units.get(ui).map_or(true, |u| u.may_check), pursuer(c) || Some(c.uid) == dsp_cop, if c.goon { 0 } else { 1 }))
        }, |i| cop_sees(world, cands[i].1, ppos, detect, det_half));
        if let Some(i) = pick {
            detector = Some((cands[i].1.state.pos.distance(ppos), cands[i].1.uid));
        }
        // (the dispatch's cop is in its tactic: the ladder takes it as the request cop)
        if detector.is_none() {
            if let Some(c) = vehicles.cars.iter().find(|c| Some(c.uid) == dsp_cop) {
                let d = c.state.pos.distance(ppos);
                if d <= detect && cop_sees(world, c, ppos, detect, det_half) {
                    detector = Some((d, c.uid));
                }
            }
        }
        let mut detected = detector.is_some();
        // (the tactic's seen test, F-16046: a target at fMovingSpeed or slower is seen; a moving one only when a probe
        // from half its height 7 m along its axis hits something - F-74492 then grows +124 and zeroes +132, or the other
        // way round, +128 keeping last frame's +124)
        // (the axis is the car's backward row, negated when the input's +44 is non-zero; the player's pad update F-56064
        // (and the base readers F-39316 / F-97149 / F-74067) copies the gearbox's gear (0 reverse, 1 neutral, 2.. forward) there: the probe looks back
        // only in reverse)
        // (the probe, F-20855: one-sided, every level object - no instance excluded, include and type flags -1, states 7;
        // any hit counts: the city, the other cars, the props and the pedestrians' capsules - `level_probe_front`)
        // (a moving target's probe also sets +88: the tactic cop whose car is the nearest hit, else none; not moving, the
        // probe is not run and +88 keeps its value)
        let t_still = tvel.length() * MPH <= tun.get("fMovingSpeed", 1.0);
        let probe_hit = !t_still && {
            let o = tpos + Vec3::Y * t_half;
            let d = if t_rev { -tfwd } else { tfwd };
            let (hit, car) = level_probe_nearest_car(world, vehicles, &self.peds, t_uid, o, d, 7.0);
            self.probe_cop = car.filter(|u| self.cops.contains(u));
            hit
        };
        let seen_now = t_still || probe_hit;
        self.seen_update(seen_now, dt);
        for c in vehicles.cars.iter().filter(|c| pursuer(c)) {
            n_cops += 1;
            let d = c.state.pos.distance(tpos);
            if d < vision {
                let from = c.state.pos + Vec3::Y * 1.2;
                let to = tpos + Vec3::Y * 1.0;
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
        // (a tactic on an AI racer is not one on the player: F-52240 asks for the player's own)
        let in_tactic = n_cops > 0 && !matches!(self.heat, Heat::None) && !ai_target;
        // (F-54211 runs F-52240 only when F-96166 allows it: not while the player drives a police car)
        let scoring = self.grace <= 0.0 && !self.player_police;
        self.grace = (self.grace - dt).max(0.0);
        let watching = scoring && (in_tactic || detected);

        // ---- offences ---------------------------------------------------------
        let mut breaking = false;
        let mut add = 0.0f32;
        // speeding: three bands above the limit
        // (F-36879: the road's limit in mph, `fSpeedLimitLow` where there is none; a band starts
        // at the larger of limit + offset and limit x (100 + percent) / 100)
        // (F-36879: the road node within 10 m of the car, F-55639; none or a limit of 0: fSpeedLimitLow)
        let limit = world.nav.as_ref().and_then(|n| match n.node_within(ppos, 10.0, false) { Some(NavNode::Road(r)) => n.zones.get(r).map(|z| z.speed_limit), _ => None }).filter(|l| *l != 0.0).map_or(t("fSpeedLimitLow", 50.0), |l| l * MPH);
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
            self.tick_law |= 0x1;
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
        // (F-88545, every scored tick and with no cop: each of the three has a condition, its started bit and one float -
        // the threshold timer and, once started, the per-second carry; `offence_gate`. Points only when a cop watches)
        // driving against the traffic (F-29334: not over an ai_shortcut polygon; the player's best lane and its tangent
        // at the kept phase; at or under fMovingSpeed x 0.447 the facing - the backward axis along the tangent -, else
        // the velocity against it)
        let wrong_cond = {
            let shortcut = crate::offroad::mesh().map_or(false, |m| m.on_flag(ppos, 8));
            let lane = vehicles.driver_lanes.get(&p.uid).and_then(|d| Some((d.best?.0, d.best_t)));
            match (shortcut, lane, world.nav.as_ref()) {
                (false, Some((l, bt)), Some(nav)) => nav.curves.get(l).map_or(false, |cv| {
                    let tan = cv.eval(bt).1.normalize_or_zero();
                    if speed < t("fMovingSpeed", 1.0) * 0.447 {
                        (-pfwd).dot(tan) > 0.0
                    } else {
                        pvel.dot(tan) < 0.0
                    }
                }),
                _ => false,
            }
        };
        // (the law record's last road +60, F-28783: the road of the target's lane, kept while it has none; FW when its
        // name starts "FW" or "XT")
        if let (Some(nav), Some(l)) = (world.nav.as_ref(), vehicles.driver_lanes.get(&p.uid).and_then(|d| d.best).map(|b| b.0)) {
            if let Some(r) = nav.lane_road(l) {
                let n = &nav.zones[r].name;
                self.fw_road = n.starts_with("FW") || n.starts_with("XT");
            }
        }
        let wrong_bit = scoring && offence_gate(wrong_cond, self.wrong_active, &mut self.sec[1], t("fWrongWayThresholdTime", 1.2), dt);
        if scoring {
            self.against_traffic = wrong_bit;
        }
        if watching && wrong_bit {
            breaking = true;
            self.offences |= LAW_WRONG_WAY;
            self.tick_law |= 0x2000;
            if !self.wrong_active {
                self.wrong_active = true;
                self.sec[1] = 0.0;
                add += t("iWrongWayStartPoints", 130.0);
            } else {
                add += self.whole_seconds(1, dt) * t("iWrongWayPerSecPoints", 25.0);
            }
        }
        // burn-outs (F-88545: the car sim's burn-out charge above 0 - the launch after it counts as well)
        let burn_cond = burn_charged(p.state.veh.as_ref().map(|v| v.burn));
        let burn_bit = scoring && offence_gate(burn_cond, self.burnout_active, &mut self.sec[2], t("fBurnOutThresholdTime", 1.5), dt);
        if watching && burn_bit {
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
        // blocking traffic (F-60011; not during a pull-over)
        let block_cond = self.heat != Heat::PullOver && block_traffic_cond(world, vehicles, p, &self.peds, &tun);
        let block_bit = scoring && offence_gate(block_cond, self.block_active, &mut self.sec[5], t("fBlockTrafficThresholdTime", 1.0), dt);
        if watching && block_bit {
            breaking = true;
            self.offences |= LAW_BLOCK_TRAFFIC;
            if !self.block_active {
                self.block_active = true;
                self.sec[5] = 0.0;
                add += t("iBlockTrafficStartPoints", 50.0);
            } else {
                add += self.whole_seconds(5, dt) * t("iBlockTrafficPerSecPoints", 25.0);
            }
        }
        // off the roads (F-90092: no polygon of the mesh with the ai_road material under the car)
        let off_road = crate::offroad::mesh().map_or(false, |m| !m.on_road(ppos));
        if watching && off_road {
            breaking = true;
            self.offences |= LAW_OFF_ROAD;
            self.tick_law |= 0x8;
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
        } else if watching {
            // (byte +140 is written only inside the detection block)
            self.jump_paid = false;
        }
        // running a red light (F-65840): only on the tick the car's nearest lane becomes a lane of a new junction. `a` is
        // the running arm while green or yellow (a junction with no signal or all red: none), `d` the way it is going
        // (slow: none): a violation unless d . a >= 0.9 - so a slow or all-red entry always counts
        if scoring {
            let j = world.nav.as_ref().zip(vehicles.driver_lanes.get(&p.uid).and_then(|d| d.best)).and_then(|(n, b)| n.lane_junction.get(b.0).copied().flatten());
            if self.last_junction != j {
                self.last_junction = None;
            }
            if let Some(n) = j.filter(|_| self.last_junction.is_none()) {
                self.last_junction = j;
                let sig = world.nav.as_ref().and_then(|nv| nv.junction_centers.get(n)).and_then(|c| vehicles.signals.junctions.iter().find(|s| (s.center - *c).length_squared() < 0.01));
                // (the arm getters F-93041 / F-75755 read J +160 / +164 only: a forced arm or forced red reaches them through `step`)
                let a = sig.map_or(Vec3::ZERO, |s| running_arm(s.timer, s.current as i32, &s.arms));
                let d = if speed > t("fMovingSpeed", 1.0) * 0.447 { -pvel / speed } else { Vec3::ZERO };
                if watching && d.dot(a) < 0.9 {
                    breaking = true;
                    self.offences |= LAW_RED_LIGHT;
                    self.tick_law |= 0x200;
                    add += t("iRedLightPoints", 250.0);
                }
            }
        }
        // the contact word (F-83713 -> F-52240, all under the detection block, one payment each a frame): a prop touched, a
        // police car run into (the aggressor only; a cop ramming a slower player scores nothing), other cars
        if watching && law_word & 1 != 0 {
            breaking = true;
            self.offences |= LAW_PROP;
            add += t("iDamagePropPoints", 250.0);
        }
        let mut hit_cop = false;
        // (F-93844: the hard byte, the largest cop value of the table over 22.35 m/s; F-26639 B says its line from the cop
        // with the largest)
        self.hard_tab = None;
        if watching && law_word & 2 != 0 {
            hit_cop = true;
            let now = vehicles.player().and_then(|c| c.state.veh.as_deref()).map_or(0.0, |v| v.agro.now);
            let tab = vehicles.player().and_then(|c| c.state.veh.as_deref()).map_or(Vec::new(), |v| v.agro.law_tab.iter().filter(|e| e.0 >> 32 == 2 && e.1 > now).map(|e| (e.0 as u32, e.2)).collect());
            if tab.iter().any(|e: &(u32, f32)| e.1 > 22.35) {
                self.hard_tab = Some(tab);
                self.offences |= LAW_POLICE_HARD;
                self.tick_law |= 0x20;
            }
            breaking = true;
            self.offences |= LAW_POLICE_VEHICLE;
            self.tick_law |= 0x40;
            add += t("iDamagePoliceVehPoints", 600.0);
        }
        if watching && law_word & 8 != 0 {
            breaking = true;
            self.offences |= LAW_VEHICLE;
            self.tick_law |= 0x80;
            add += t("iDamageOtherVehPoints", 130.0);
        }
        self.last_damage = damage;

        // (AddPoints, F-19359, for every offence, the decay and the per-second growth: the law tactic on the player - none
        // when the tactic is on an AI racer - drops the points when its may-escalate byte is 0, and caps the score: a pull-over
        // at iPursuitLawBreakScore - 1 until its bullhorn has sounded and F-98647 holds, a Pursuit / Double Pursuit at iMax - 1
        // until its vt+24 holds - not in its Delay, no radio line, the byte, not searching or arresting, no arrest timer)
        let max_score = t("iMaxLawBreakingScore", 1000.0);
        let pull = t("iPullOverLawBreakScore", 250.0);
        let (searching, arresting) = (self.heat == Heat::Search, self.heat == Heat::Busted);
        let law_tac = (self.kind != TacKind::None && self.target.is_none()).then(|| LawTactic {
            ty: self.kind.ty(),
            may: self.may_escalate,
            sounded: self.bullhorn.is_some(),
            escalates: match self.kind {
                TacKind::PullOver => pullover_may_escalate(
                    self.may_escalate,
                    self.arrest,
                    self.bullhorn.unwrap_or(0.0),
                    self.ticket_hold.is_some() || self.ticketing,
                ),
                TacKind::Pursuit => {
                    pursuit_may_escalate(self.flee_hold > 0.0, self.radio_busy, self.may_escalate, searching, arresting, self.arrest)
                }
                _ => tactic_may_escalate(self.radio_busy, self.may_escalate, searching, arresting, self.arrest),
            },
        });
        let (max_i, pursuit_i) = (max_score as i32, t("iPursuitLawBreakScore", 600.0) as i32);
        let add_points_now = |score: f32, points: f32| {
            add_points(score as i32, points as i32, law_tac, max_i, pursuit_i).map_or(score, |v| v as f32)
        };
        if breaking {
            self.score = add_points_now(self.score, add);
        }
        if scoring {
            // (F-52240 after the offences: under iPullOverLawBreakScore the score falls iNoLawBrokenPerSecPoints a
            // whole second while no offence is counted - iUndetectedPerSecPoints is read by nothing -; from it on it
            // climbs iPullOverPerSecPoints a whole second, offences or not, and never falls)
            if self.score > 0.0 && self.score < pull {
                if !breaking {
                    let n = self.whole_seconds(5, dt);
                    if n > 0.0 {
                        self.score = add_points_now(self.score, -n * t("iNoLawBrokenPerSecPoints", 10.0));
                    }
                    if self.score <= 0.0 {
                        self.offences = 0;
                    }
                }
            } else if self.score >= pull && self.score < max_score {
                let n = self.whole_seconds(6, dt);
                if n > 0.0 {
                    self.score = add_points_now(self.score, n * t("iPullOverPerSecPoints", 10.0));
                }
            }
        }

        // ---- state machine ----------------------------------------------------
        let mut event = pre_event;
        // (the hit-a-cop steal, the law step F-52240's race branch: in a race, the player in no tactic of his own and a
        // police car hit this tick (record bits 0x20 / 0x40, scored only when watched) - the Arrow Trap that may retarget
        // (+30) and chases an AI racer ends and a new one starts on the player, that trap's cop as its request cop)
        if ai_target && self.in_race && hit_cop && self.kind == TacKind::ArrowTrap && self.tac_retarget && !goons {
            let first = self.units.iter().filter_map(|u| Self::unit_car(vehicles, u.id)).map(|i| vehicles.cars[i].uid).find(|u| self.cops.contains(u)).or(self.cops.first().copied());
            log::info!("police: a police car hit - the Arrow Trap on AI racer {:?} turns on the player", self.target);
            self.end_tactic(false);
            if let Some(cops) = self.engage(gpu, world, vehicles, TacKind::ArrowTrap.cops(), first, ppos, pfwd) {
                self.start_tactic(TacKind::ArrowTrap, cops, true, t("fTrackTgtTime", 25.0));
                self.heat = Heat::Pursuit;
                self.last_known = ppos;
                self.last_known_v = pvel;
                self.fine = self.law_fines();
                self.lost = 0.0;
                event = Some(Event::Pursuit);
                self.start_report = Some("REPORT_HIT_COP_CAR_HARD");
            }
        }
        // (from here on the tactic's target)
        let ai_target = self.target.is_some();
        let (ppos, pvel, pfwd, pright, damage, burning, bike_down) = match self.target_car(vehicles) {
            Some(c) => (c.state.pos, c.state.vel, c.state.forward(), c.state.right(), c.state.damage, burn_charged(c.state.veh.as_ref().map(|v| v.burn)), c.model.bike && c.state.wipeout > 0.0),
            None => (tpos, tvel, tfwd, tright, tdamage, t_burning, t_bike_down),
        };
        let speed = pvel.length();
        let mph = speed * MPH;
        let arrest_speed = arrest_ms(t("fArrestTgtThresholdSpd", 10.0));
        // (the tactic's radius vt+32: fPullOverDist in a pull-over (F-66453), fSafeDist in the others (F-54202))
        let arr_r = if self.heat == Heat::PullOver { t("fPullOverDist", 15.0) } else { t("fSafeDist", 30.0) };
        let bike_r = t("fBikerWipeoutArrestDist", 50.0);
        let seer = |r: f32| vehicles.cars.iter().filter(|c| pursuer(c)).any(|c| cop_sees(world, c, ppos, r, 0.0));
        let (seer_r1, seer_bike) = (seer(arr_r + 1.0), bike_down && seer(bike_r));
        // (F-29443: some tactic cop not damaged out with its car strictly within the radius of the target, no sight needed)
        let cop_in = vehicles.cars.iter().filter(|c| pursuer(c)).any(|c| !damaged_out(c.state.damage) && within_sq(ppos, c.state.pos, arr_r));
        let delay = self.kind == TacKind::Pursuit && self.flee_hold > 0.0;
        // (the Delay's exit F-25831 runs the update the Delay ends; the Search's clock +16 > 0 is the target lost, F-77220)
        let leaving = self.arrest_delay && !delay;
        let lost = search_clock_runs(self.search);
        let ain = |slow: bool, search: bool| ArrestIn {
            seer_r1,
            seer_bike,
            delay,
            leaving,
            search,
            lost: search && lost,
            cop_in,
            refresh: true,
            slow,
            bike_down,
            wrecked: damaged_out(damage),
            burning,
            pos: ppos,
        };
        match self.heat {
            Heat::None => {
                // (F-52240's ladder needs a detecting cop, the request cop: at iMaxLawBreakingScore an Arrow Trap, from
                // iPursuitLawBreakScore a Pursuit, from iPullOverLawBreakScore a Pull Over; StartTactic, F-33550, raises
                // the score to the tactic's floor - the zone's scripted chase too, its cop the nearest)
                let zone = std::mem::take(&mut self.zone_go);
                let first = detector.map(|d| d.1);
                // (the ladder is F-52240's: none for a player in a police car)
                let detected = detected && !self.player_police;
                let tt = t("fTrackTgtTime", 25.0);
                let kind = if detected && self.score >= max_score {
                    TacKind::ArrowTrap
                } else if zone {
                    // (the zone's start, F-71212 -> StartTactic type 4: the Arrow Trap, its floor 1000 on the player)
                    TacKind::ArrowTrap
                } else if detected && self.score >= t("iPursuitLawBreakScore", 600.0) {
                    TacKind::Pursuit
                } else if detected && self.score >= pull {
                    TacKind::PullOver
                } else {
                    TacKind::None
                };
                if kind != TacKind::None {
                    // (F-52240, the detecting cop's three starts: the speech request F-70230 before StartTactic, whether
                    // that starts or not - not the zone's own start, F-71212 says its own line)
                    if detected && self.score >= pull {
                        self.begin_cue = Some(self.target);
                        // (its Arrow Trap / Pursuit report is the default one with +8 = the record's +100 & +96)
                        if kind != TacKind::PullOver {
                            self.report_bits = Some(self.tick_law);
                        }
                    }
                    if let Some(cops) = self.engage(gpu, world, vehicles, kind.cops(), if zone && !detected { None } else { first }, ppos, pfwd) {
                        self.start_tactic(kind, cops, true, tt);
                        if kind == TacKind::PullOver {
                            self.heat = Heat::PullOver;
                            self.bullhorn = None;
                            self.ticket_hold = None;
                            self.reset_arrest();
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
                if let Some(hold) = self.ticket_hold.as_mut() {
                    // (StateTicket: the cop halted, fArrestStateTime - 0.5 s more when the car is damaged out -,
                    // then the tactic's result and COP_PULLOVER)
                    if ticket_tick(hold, dt) {
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
                } else if self.arrest_step(ain(speed < arrest_speed, false), t("fPullOverDist", 15.0), t("fArrestTime", 1.0), t("fBikerWipeoutArrestDist", 50.0), arrest_speed * dt, dt) {
                    // (the pull-over's radius fPullOverDist, F-66453)
                    if self.ticket_hold.is_none() {
                        // (entered in the update the arrest test answers; its first tick there sees 0, then dt is added)
                        self.ticket_hold = Some((dt, t("fArrestStateTime", 1.75) + if damaged_out(damage) { 0.5 } else { 0.0 }));
                        // (what the ticket is for - the panel lists it: 1 reckless driving,
                        // 2 traffic violation, 4 destruction of property)
                        self.ticket_for = (if self.offences & (LAW_BURNOUT | LAW_VEHICLE | LAW_POLICE_VEHICLE) != 0 { 1 } else { 0 }) | (if self.offences & (LAW_SPEED | LAW_WRONG_WAY | LAW_RED_LIGHT) != 0 { 2 } else { 0 }) | (if self.offences & LAW_PROP != 0 { 4 } else { 0 });
                    }
                }
            }
            Heat::Pursuit => {
                if !ai_target {
                    self.fine = self.law_fines();
                }
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
                            self.srch.push(search_start(u, pts, false, angle));
                            // (the Search state's enter sends command 7 with its block, F-60530 + the angle, through the brain
                            // command F-83687; the unseen and cone times are kept)
                            if let Some(tc) = self.tacs.get_mut(&u) {
                                brain_command(&mut tc.brain, 7, Some(&search_block(false, angle)), 0);
                            }
                        }
                    }
                }
                // (the second-cop upgrade, F-22881: a free patrolling cop that detects the pursued target joins - a Pursuit
                // becomes a Double Pursuit, a Double Pursuit an Arrow Trap, the joining cop first - when the tactic may
                // escalate; at iMaxLawBreakingScore the ladder makes it an Arrow Trap with its own cop 0)
                // (the port's detecting cop is the player's: none for an AI racer's tactic, an Arrow Trap already)
                if self.heat == Heat::Pursuit && self.may_escalate && !goons && !ai_target {
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
                // (the pursuit's radius fSafeDist, F-54202; every tactic's Arrest ask needs the target's vt+40, the player:
                // an AI racer is never arrested - F-68176 / the Arrow Trap's update)
                if !ai_target && self.arrest_step(ain(speed < arrest_speed, false), t("fSafeDist", 30.0), t("fArrestTime", 1.0), t("fBikerWipeoutArrestDist", 50.0), arrest_speed * dt, dt) {
                    {
                        self.heat = Heat::Busted;
                        self.arrest_posted = false;
                        // (F-56938: the arrest ends after fArrestStateTime, 0.5 s longer when the
                        // target is damaged out - the enter state F-69250 only stops a car whose damage
                        // points are under its MaxDamage and then sets the tactic byte +68)
                        self.busted_timer = t("fArrestStateTime", 1.75) + if damaged_out(damage) { 0.5 } else { 0.0 };
                        event = Some(Event::Busted(self.fine as i64));
                    }
                }
            }
            Heat::Search => {
                // (the Pursuit's state 6: the target lost - no arrest, +140 / +144 zeroed and +48 kept, F-63286)
                if !ai_target {
                    self.arrest_step(ain(speed < arrest_speed, true), t("fSafeDist", 30.0), t("fArrestTime", 1.0), t("fBikerWipeoutArrestDist", 50.0), arrest_speed * dt, dt);
                }
                // (the second-cop upgrade runs for a searching tactic too: F-22881 tests only the tactic's +28 - a free
                // cop that sees the target makes the Pursuit a Double Pursuit, that an Arrow Trap, in CatchUp)
                if let (true, Some((_, u))) = (self.may_escalate && !goons && !ai_target && matches!(self.kind, TacKind::Pursuit | TacKind::PursuitDbl), detector) {
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
                        self.lost_ok = false;
                        self.search += dt;
                    } else {
                        self.track_t = 20.0;
                        self.lost_ok = false;
                        if !self.goons {
                            self.radio_cue.push(if self.search / t("fSearchMaxTime", 35.0) < 0.5 { "REPORT_SUSPECT_SPOTTED" } else { "REPORT_SUSPECT_REACQUIRE" }.into());
                        }
                        self.search = 0.0;
                        self.heat = Heat::Pursuit;
                    }
                    if self.heat == Heat::Pursuit {
                        self.eyes.clear();
                        self.srch.clear();
                        // (the tactic's machine starts again; each cop's brain keeps what its last command wrote - Search's
                        // 7 and its side / angle - until the next command, as the original's brains do: F-83687's previous
                        // command and the machine reset read it)
                        for tc in self.tacs.values_mut() {
                            *tc = CopTac { brain: tc.brain, ..Default::default() };
                        }
                        self.sq = Squad { hyst: 2.0, ..Default::default() };
                    }
                } else {
                    if started {
                        self.search += dt;
                    }
                    if self.search > t("fSearchMaxTime", 35.0) {
                        // (result 2, F-22530: for an AI racer the radio's lines alone - no record to clear, no reward, no
                        // grace; in a race SetAllPoliceUnderground(1, 1), F-36452, for any target)
                        if ai_target {
                            // (in a race F-36452 clears the player's record as well)
                            if self.in_race { self.clear() } else { self.end_tactic(true) }
                            event = Some(Event::LostAi);
                        } else {
                            self.clear();
                            self.grace = 5.0;
                            event = Some(Event::Escaped);
                        }
                        if self.in_race {
                            self.units_down(vehicles);
                        }
                    }
                }
                // (the race retarget, F-22530: a race current, the tactic's +30 (the manager's +221: 1 but in BEAT_SH_04B,
                // `set_chase_properties`) and the target lost - the clock running: the nearest racer a tactic cop sees within 300 m (F-94676,
                // no cone), not the target - the players first, the AI racers only when no player is seen. The tactic
                // ends with vt+12(0) and an Arrow Trap starts on that racer, the seeing cop its request cop,
                // REPORT_VIOLATION_PURSUIT)
                if self.heat == Heat::Search && self.in_race && self.search > 0.0 && self.tac_retarget && !goons {
                    let cur = self.target;
                    let cops: Vec<(u32, Vec3, &crate::vehicles::CarInstance)> = self.cops.iter().filter_map(|u| vehicles.cars.iter().find(|c| c.uid == *u)).map(|c| (c.uid, c.state.pos, c)).collect();
                    let pick = |want_player: bool| -> Option<(f32, u32, u32)> {
                        let mut best: Option<(f32, u32, u32)> = None;
                        for r in self.racer_uids.iter().filter_map(|u| vehicles.cars.iter().find(|c| c.uid == *u)) {
                            if r.is_player != want_player || (if r.is_player { cur.is_none() } else { cur == Some(r.uid) }) {
                                continue;
                            }
                            for &(cu, cp, c) in &cops {
                                let d2 = cp.distance_squared(r.state.pos);
                                if best.map_or(true, |b| d2 < b.0) && cop_sees(world, c, r.state.pos, 300.0, 0.0) {
                                    best = Some((d2, cu, r.uid));
                                }
                            }
                        }
                        best
                    };
                    let found = pick(true).or_else(|| pick(false));
                    if let Some((_, cop, r)) = found {
                        let car = vehicles.cars.iter().find(|c| c.uid == r).map(|c| (c.is_player, c.state.pos, c.state.forward(), c.state.vel));
                        if let Some((is_player, pos, fwd, vel)) = car {
                            log::info!("police: the target lost - cruiser {} sees racer {} (player {}): a new Arrow Trap", cop, r, is_player);
                            self.end_tactic(false);
                            self.target = (!is_player).then_some(r);
                            if let Some(cops) = self.engage(gpu, world, vehicles, TacKind::ArrowTrap.cops(), Some(cop), pos, fwd) {
                                self.start_tactic(TacKind::ArrowTrap, cops, true, t("fTrackTgtTime", 25.0));
                                self.heat = Heat::Pursuit;
                                self.last_known = pos;
                                self.last_known_v = vel;
                                if is_player {
                                    self.fine = self.law_fines();
                                    event = Some(Event::Pursuit);
                                } else {
                                    self.radio_cue.push("REPORT_VIOLATION_PURSUIT".into());
                                }
                            } else {
                                self.target = None;
                            }
                        }
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
                    // (CopArrest.sco 0xa9f / 0xaa8: SetAllPoliceActive(0), SetAllPoliceUnderground(1, 1) - every unit
                    // underground until (0, 1) at the script's end, F-30655's CutSceneDone. SetAllPoliceActive(0)
                    // (F-11309 -> F-75974) only clears +1072 / +1073; no cop near the arrest is moved or removed by
                    // anything else. The goons' path has no form: their end comes at once)
                    self.units_down(vehicles);
                    if goons {
                        self.units_up();
                    }
                }
            }
        }

        // (result 3, a tactic cop gone: the manager loses the police, F-22530)
        if gone && matches!(self.heat, Heat::Pursuit | Heat::Search | Heat::PullOver) {
            log::info!("police: a cruiser of the {:?} is gone - the police are lost", self.kind);
            if ai_target {
                if self.in_race { self.clear() } else { self.end_tactic(true) }
                event = Some(Event::LostAi);
            } else {
                self.clear();
                self.grace = 5.0;
                event = Some(Event::Escaped);
            }
            if self.in_race {
                self.units_down(vehicles);
            }
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
                    // (StartTactic's strip text for a Dispatch, F-76838 kind 2: the investigation)
                    self.hud_msgs.push(2);
                    self.dsp = Some(Dispatch { cop, st: 0, t: 0.0, loc: ppos, call, crumbs: Vec::new(), crumb_t: 1.0, seen_t: 0.0, trail: false, routed: false, arrived: false, near_t: 0.0, go_t: 0.0, next: None, brain: BrainCmd::default() });
                }
            }
        }
        if self.kind != TacKind::Dispatch || self.heat != Heat::None {
            self.dsp = None;
        }
        if let Some(mut d) = self.dsp.take() {
            let cop = vehicles.cars.iter().find(|c| c.uid == d.cop);
            let mut result = 0;
            // (the player's trail: a point a second, 15 at most, while the cop has not reached the place)
            if d.st < 3 && d.crumbs.len() < 15 && d.crumb_t > 0.0 {
                d.crumb_t -= dt;
                if d.crumb_t <= 0.0 {
                    d.crumb_t = 1.0;
                    d.crumbs.push(ppos);
                    // (F-87106: for a racer target the trail stops once a crumb is taken while its +640 - the lane its
                    // lane locator found this frame, F-80793 - is set: normally at the first crumb)
                    if vehicles.player().is_some_and(|p| vehicles.driver_lanes.get(&p.uid).is_some_and(|l| l.best.is_some())) {
                        d.crumb_t = 0.0;
                    }
                }
            }
            match cop {
                None => result = 3,
                Some(c) => {
                    // (Respond / WaitAtLocation: the cop sees the target within fRdoVicinityDist and fDetectHalfAngle,
                    // line of sight, over 0.5 s; F-87106)
                    if d.st >= 2 && cop_sees(world, c, ppos, t("fRdoVicinityDist", 100.0), t("fDetectHalfAngle", 180.0)) {
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
                // (F-95322: the state asked for in the last update is entered first, F-43677; then the state's test runs on
                // its time, which grows by dt after it, F-88920)
                match dispatch_enter(&mut d) {
                    // (Dispatch F-44635: the DISPATCH line of the trigger's sound record)
                    Some(1) => self.radio_cue.push(d.call.clone()),
                    // (Respond F-81858: REPORT_ACCEPT_CALL, GoToPosition to the place, siren on)
                    Some(2) => {
                        self.radio_cue.push("REPORT_ACCEPT_CALL".into());
                        brain_command(&mut d.brain, 8, Some(&respond_block(d.loc)), 0);
                    }
                    // (WaitAtLocation F-52732: along the player's trail, else a search at once, +-pi/2 at random)
                    Some(3) if !d.trail => self.dispatch_search(world, vehicles, d.cop, t("fMinSetGoalTime", 1.0), &mut d.brain),
                    _ => {}
                }
                dispatch_ask(&mut d, t("fDispatchRdoDelayTime", 3.0), self.speech_busy);
                // (F-92001: fDispatchLocWaitTime at the place ends it; at the trail's last crumb a search)
                if d.st == 3 {
                    if d.t > t("fDispatchLocWaitTime", 30.0) {
                        result = 2;
                    } else if d.trail && d.arrived {
                        d.trail = false;
                        if let Some(&last) = d.crumbs.last() {
                            self.last_known = last;
                        }
                        self.dispatch_search(world, vehicles, d.cop, t("fMinSetGoalTime", 1.0), &mut d.brain);
                    }
                }
                d.t += dt;
            }
            match result {
                // (F-22530: a dispatch's result 1 starts a Pursuit with its cop)
                1 => {
                    log::info!("police: dispatch cruiser {} sees the player - pursuit", d.cop);
                    self.srch.clear();
                    self.start_tactic(TacKind::Pursuit, vec![d.cop], true, t("fTrackTgtTime", 25.0));
                    self.tacs.insert(d.cop, CopTac { brain: d.brain, ..Default::default() });
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
                    event = Some(Event::DispatchLost);
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
        let t_moving = moving(speed, t("fMovingSpeed", 1.0));
        // (F-42951: false only for a group cop - not a goon - while a race runs: then the brains plan the plain way and
        // the squad never swipes)
        let ed78 = !(self.in_race && !goons);
        let mut swipes: HashMap<u32, crate::ai::Overrides> = HashMap::new();
        if self.kind.squad() && self.heat == Heat::Pursuit && self.flee_hold <= 0.0 && self.cops.len() == self.kind.cops() {
            let arrow = self.kind == TacKind::ArrowTrap;
            // (closing(r): the cop moving, faster than the target and heading at it within 25.8 deg, within r)
            let closing = |c: &crate::vehicles::CarInstance, r: f32| closing_in(ppos, c.state.pos, speed, c.state.vel.length(), c.state.vel, r, t("fMovingSpeed", 1.0));
            let any_safe = vehicles.cars.iter().filter(|c| pursuer(c)).any(|c| c.state.pos.distance(ppos) < safe);
            // (the formation, F-80532 / F-21534 in squad_formation: the cops' brains as their CopTac - AlongSide 5 with its
            // side, CatchUp the Pursue 4 - and the brains' route points; the target matrix's third row is -forward)
            let formation = |sq: &mut Squad, tacs: &mut HashMap<u32, CopTac>, vehicles: &Vehicles, dt: f32, enter: bool| -> bool {
                let car = |u: u32| vehicles.cars.iter().find(|c| c.uid == u);
                let pos: Vec<Vec3> = members.iter().map(|&u| car(u).map_or(Vec3::ZERO, |c| c.state.pos)).collect();
                let route: Vec<Option<Vec3>> = members.iter()
                    .map(|&u| car(u).and_then(|c| c.ai.as_ref()).and_then(|a| a.brain.0.as_ref()).and_then(|b| b.heading_for().1)).collect();
                let code = |st: Tac| match st {
                    Tac::AlongSide => 5,
                    Tac::CatchUp => 4,
                    _ => 0,
                };
                let mut brains: Vec<(u8, bool)> = members.iter().map(|u| tacs.get(u).map_or((0, false), |tc| (code(tc.st), tc.side))).collect();
                let was = brains.clone();
                let index = |u: Option<u32>| u.and_then(|u| members.iter().position(|&m| m == u));
                let mut slots = sq.slots.map(index);
                let sent = squad_formation(&mut sq.hyst, &mut slots, arrow, &mut brains, &pos, &route, ppos, pright, -pfwd, along, dt, enter);
                for k in (0..members.len()).filter(|&k| brains[k] != was[k]) {
                    let st = if brains[k].0 == 5 { Tac::AlongSide } else { Tac::CatchUp };
                    let tc = tacs.entry(members[k]).or_default();
                    *tc = CopTac { st, side: brains[k].1, cmd: brains[k].0, sent: true, brain: tc.brain, ..Default::default() };
                }
                sq.slots = slots.map(|k| k.map(|k| members[k]));
                sent
            };
            let mut sq = std::mem::take(&mut self.sq);
            let mut tacs = std::mem::take(&mut self.tacs);
            // the flank swipes (only when F-42951 holds for cop A; squad_swipes), on each cop's tests against the target
            let geo = |u: u32| -> Option<SwipeGeo> {
                let c = vehicles.cars.iter().find(|c| c.uid == u)?;
                Some(swipe_geo(c.state.pos, c.state.right(), -c.state.forward(), c.state.vel, ppos, pvel))
            };
            let head_on_dist = t("fHeadOnBlockDist", 60.0);
            let fast = mph > t("fArrestTgtThresholdSpd", 10.0);
            let mut cues: Vec<String> = Vec::new();
            let mut cx = (tacs, swipes, &mut cues);
            type Cx<'a> = (HashMap<u32, CopTac>, HashMap<u32, crate::ai::Overrides>, &'a mut Vec<String>);
            let halted = squad_machine(&mut sq, &mut cx, arrow, any_safe, t_moving, fast, dt, &mut |sq: &mut Squad, cx: &mut Cx, dt, enter| formation(sq, &mut cx.0, vehicles, dt, enter), &mut |sq: &mut Squad, cx: &mut Cx| {
                if !ed78 {
                    return;
                }
                let g: Vec<Option<SwipeGeo>> = members.iter().map(|&u| geo(u)).collect();
                let head_on: Vec<bool> = members.iter().map(|u| cx.0.get(u).map_or(false, |tc| tc.head_on)).collect();
                let index = |u: Option<u32>| u.and_then(|u| members.iter().position(|&m| m == u));
                let mut ovr = vec![crate::ai::Overrides::default(); members.len()];
                if squad_swipes(sq, arrow, dt, index(sq.slots[0]), index(sq.slots[1]), &g, &head_on, head_on_dist, &mut ovr) && !goons {
                    cx.2.push("REPORT_MANEUVER_CLOSE".into());
                }
                for (k, o) in ovr.into_iter().enumerate() {
                    if o.steer.is_some() || o.brake.is_some() || o.handbrake.is_some() {
                        cx.1.insert(members[k], o);
                    }
                }
            });
            let (mut tacs, s, _) = cx;
            swipes = s;
            self.radio_cue.extend(cues);
            if halted {
                // Halt (F-58305 / F-16041): the cops within fSafeDist Stop again, x 0.5 when closing (the per-cop timer +16
                // and the brain's +568 / +688 test are not ported)
                for c in vehicles.cars.iter_mut().filter(|c| pursuer(c) && c.state.pos.distance(ppos) < safe) {
                    let tc = tacs.entry(c.uid).or_default();
                    if tc.st != Tac::Halt {
                        *tc = CopTac { st: Tac::Halt, cmd: 1, sent: true, brain: tc.brain, ..Default::default() };
                    }
                    if closing(c, 0.0) {
                        c.state.vel *= 0.5;
                    }
                }
            }
            self.sq = sq;
            self.tacs = tacs;
        }

        // ---- cruisers ---------------------------------------------------------
        let chasing = matches!(self.heat, Heat::Pursuit | Heat::Search | Heat::PullOver | Heat::Busted);
        delay_step(&mut self.flee_hold, &mut self.flee_t, &mut self.flee_ask, dt);
        if !matches!(self.heat, Heat::Pursuit | Heat::PullOver | Heat::Search) {
            self.tacs.clear();
        }
        // (in the Search the tactic's other cops keep the command they had: their states do not change)
        let searching = self.heat == Heat::Search;
        let tactic = matches!(self.heat, Heat::Pursuit | Heat::PullOver | Heat::Search) && self.flee_hold <= 0.0;
        let squad = self.kind.squad();
        let pull = self.heat == Heat::PullOver;
        let period = t("fMinSetGoalTime", 1.0);
        let last_tspeed = self.last_tspeed;
        self.last_tspeed = speed;
        // (every cop of the tactic within r of the target: F-90575, F-36876)
        // (the squared distance below r x r, F-39842)
        let all_d2: Vec<f32> = vehicles.cars.iter().filter(|c| pursuer(c)).map(|c| arrest_d2(c.state.pos, ppos)).collect();
        let within = |r: f32| all_d2.iter().all(|&d| d < r * r);
        // (the pull-over's: within fPullOverDist - 1, the tactic's slot 8 F-66453 less 1.0)
        let all_halt = within(t("fPullOverDist", 15.0) - 1.0);
        let on_road = |q: Vec3| crate::offroad::mesh().map_or(true, |m| m.on_road(q));
        let on_mesh = |q: Vec3| crate::offroad::mesh().map_or(true, |m| m.on_mesh(q));
        for c in vehicles.cars.iter_mut().filter(|c| pursuer(c) || Some(c.uid) == dsp_cop) {
            let is_dsp = Some(c.uid) == dsp_cop && !pursuer(c);
            // (the head-on test's sight: this cop's own, 2 m up, no range and no cone)
            let own_sight = tactic && cop_sees(world, c, ppos, 0.0, 0.0);
            let Some(ai) = c.ai.as_mut() else { continue };
            ai.chase_exact = false;
            if self.flee_hold > 0.0 && self.heat == Heat::Pursuit && !is_dsp {
                // (the Delay state: brain command 1, halt, until its time is up)
                ai.chase = Some((c.state.pos, Vec3::ZERO));
                ai.ovr = Some(crate::ai::Overrides { brake: Some(1.0), ..Default::default() });
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
                // (GoToPosition's tick F-16378 on the brain's +580 and its state time, then the time; the port's own driver
                // without a racing tune: within 8 m of the end)
                d.near_t = if goal.is_some_and(|g| g.distance_squared(c.state.pos) < 64.0) { d.near_t + dt } else { 0.0 };
                let done = ai.brain.0.as_ref().map_or(c.state.pos.distance(end) < 8.0, |b| b.route_done());
                d.arrived = go_to_arrived(d.arrived, d.near_t, d.go_t, done);
                d.go_t += dt;
                ai.chase = Some((goal.unwrap_or(end), Vec3::ZERO));
                ai.chase_exact = true;
                ai.target_speed = 85.0;
            } else if tactic {
                // ---- the tactic's state and its brain (docs/reference/hookmen/pursuit_tactics.md) ----
                if self.heat == Heat::Pursuit && self.kind != TacKind::PullOver {
                    self.brain_tvel.insert(c.uid, pvel);
                }
                let tac = self.tacs.entry(c.uid).or_default();
                let (cpos, cvel, cfwd, cright) = (c.state.pos, c.state.vel, c.state.forward(), c.state.right());
                let cspeed = cvel.length();
                let to = ppos - cpos;
                let d = to.length();
                // (the brain's predicates, F-74140's helpers: in front of the cop, on its right, going the same way,
                // facing the same way, the angle off the cop's right axis, and "closing in": the cop moving, faster
                // than the target and heading at it within 25.8 deg)
                // (in the original's float order, as the swipes take them: `swipe_geo`)
                let geo = swipe_geo(cpos, cright, -cfwd, cvel, ppos, pvel);
                let (ahead, on_right, same_dir) = (geo.ahead, geo.on_right, geo.same_dir);
                let same_facing = same_facing(pfwd, cfwd);
                let closing = |r: f32| closing_in(ppos, cpos, speed, cspeed, cvel, r, t("fMovingSpeed", 1.0));
                // (head-on, +774: in front, coming the other way, seen, travel directions more than fHeadOnAngle apart)
                let tdir = if t_moving { pvel.normalize_or_zero() } else { pfwd };
                let cdir = if cspeed > moving_v { cvel / cspeed } else { cfwd };
                let was_head_on = tac.head_on;
                tac.head_on = ahead && !same_dir && !same_facing && own_sight && tdir.angle_between(cdir).to_degrees() > t("fHeadOnAngle", 145.0);
                // (F-14848: the points beside the target over the cop filter's mesh, then over the road; `side_pick`)
                let pick_side = || {
                    let (p1, p2) = (ppos + pright * along, ppos - pright * along);
                    side_pick(on_mesh(p1), on_mesh(p2), on_road(p1), on_road(p2), on_right)
                };
                let st = tac.st;
                // (Halt's steer away as its tick writes it, F-84018: from the update after it was taken)
                let halt_steer0 = if st == Tac::Halt { tac.halt_steer } else { 0.0 };
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
                // (the brain ticks before the tactics (docs/reference/hookmen/brain_tactic_order.md): it plans and
                // overrides by the command it holds - the one the tactic sent last frame; this frame's goes in below)
                let bcmd = tac.brain.cmd;
                let plans = matches!(bcmd, 3 | 4 | 5) && !searching;
                let per = if tac.head_on { 0.5 * period } else { period };
                tac.replan = (tac.replan - dt).max(0.0);
                let (empty, done) = ai.brain.0.as_ref().map_or((true, false), |b| (b.goal_count() == 0, b.route_done()));
                let turned = tac.plan_fwd != Vec3::ZERO && tac.plan_fwd.angle_between(pfwd) > 0.5236;
                let changed = (speed - tac.plan_speed).abs() > 6.705;
                let ended_head_on = was_head_on && !tac.head_on;
                let mut seen = ppos;
                if nav_snap(&mut seen) {
                    tac.last_seen = seen;
                }
                // (F-67303's order: the countdown out, the route's last goal reached and a head-on just over plan
                // anew; else, with a law record, under 0.3 x fMinSetGoalTime (not halved head-on) left, the target
                // turned more than 30 deg since the last plan or its speed changed by 6.705 m/s re-aims)
                let normal = tac.replan <= 0.0 || done || ended_head_on;
                let quick = !normal && tac.replan < 0.3 * period && (turned || changed);
                if plans && (normal || quick || empty) {
                    // (the target's lane tangent, its +624, at its kept phase)
                    let lane_tan = world.nav.as_ref().zip(lane_of_player).and_then(|(n, (l, bt))| Some(n.curves.get(l)?.eval(bt).1));
                    let pin = PlanIn { bcmd: bcmd as u8, ed78, head_on: tac.head_on, quick, turned, t_moving, same_dir, ahead, wrong_way: self.against_traffic, lane_tan, side: tac.brain.side as u8, ppos, pvel, pfwd, pright, cpos, last_seen: tac.last_seen, speed, d, safe, along, per: period, moving_speed: t("fMovingSpeed", 1.0), pull_dist: t("fPullOverDist", 15.0) };
                    let (g, v) = cop_plan(world.nav.as_ref(), &pin);
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
                // (before the brain's first plan - the tactic's first command is not in yet - the cop keeps its chase)
                if tac.goals != [Vec3::ZERO; 2] {
                    let goal = if cpos.distance(tac.goals[0]) < 5.0 { tac.goals[1] } else { tac.goals[0] };
                    ai.chase = Some((goal, Vec3::ZERO));
                    ai.chase_exact = true;
                    ai.target_speed = 85.0;
                }
                // ---- the brain's overrides (F-88118 puts them over the driver's input) ----
                ai.ovr = {
                    // the brain state's overrides by its command (brain_override; the Search, 7, has none; a squad's cops get theirs
                    // from the formation, AlongSide 5 / Pursue 4, and its Halt, Stop 1)
                    let w = tac.brain.stop;
                    let stop = StopParams { hb: w[0] >> 24 != 0, amount: f32::from_bits(w[1]), throttle: f32::from_bits(w[2]), steer: f32::from_bits(w[3]) };
                    let (dx, dy, dz) = (ppos.x - cpos.x, ppos.y - cpos.y, ppos.z - cpos.z);
                    let bs = BrainSees { moving: t_moving, ahead, same_dir, ed78, cspeed, tspeed: speed, d2: fmas(dx, dx, fmas(dz, dz, dy * dy)), safe };
                    let o = brain_override(if searching { 7 } else { bcmd as u8 }, tac.brain.side_d, &stop, &bs);
                    let mut v = crate::ai::Overrides {
                        throttle: (o[0] != -1.0).then_some(o[0]),
                        brake: (o[1] != -1.0).then_some(o[1]),
                        handbrake: (o[2] != -1.0).then_some(o[2] > 0.0),
                        steer: (o[3] != -1000.0).then_some(o[3]),
                    };
                    if !pull && st == Tac::Halt && halt_steer0 != 0.0 {
                        v.steer = Some(halt_steer0);
                    }
                    (v.throttle.is_some() || v.brake.is_some() || v.handbrake.is_some() || v.steer.is_some()).then_some(v)
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
                // (then the common update F-63286: full brake for the cop the target's probe hit, +88)
                if self.probe_cop == Some(c.uid) {
                    ai.ovr = Some(crate::ai::Overrides { brake: Some(1.0), ..ai.ovr.unwrap_or_default() });
                }
                // ---- the tactic's machine, after the brain: its command acts at the next frame's brain tick ----
                if searching || squad {
                    // (no tactic state changes during the Search; a squad's commands come from its FSM)
                    tac.t += dt;
                } else if pull {
                    // pull-over (F-36876, pullover_machine; the re-pick gate's tactic +36 is not ported: open)
                    let pull_r = t("fPullOverDist", 15.0);
                    let (dx, dy, dz) = (ppos.x - cpos.x, ppos.y - cpos.y, ppos.z - cpos.z);
                    let sees = PullSees {
                        all_halt,
                        moving: t_moving,
                        closing0: closing(0.0),
                        closing7: closing(7.0),
                        closing50: closing(50.0),
                        near50: d < 50.0,
                        near_pull: d < pull_r,
                        pick: pick_side(),
                        gate: true,
                        cspeed,
                        d2: fmas(dx, dx, fmas(dz, dz, dy * dy)),
                    };
                    c.state.vel *= pullover_machine(tac, &sees, dt);
                } else {
                    // pursuit (F-68032, pursuit_machine; the re-pick gate's tactic +36 is not ported: open)
                    let sees = pursuit_sees(&geo, &all_d2, safe, along, -cfwd, -pfwd, speed, t("fMovingSpeed", 1.0), closing(safe), pick_side());
                    pursuit_machine(tac, &sees, t("fArrestTgtThresholdSpd", 10.0), last_tspeed, dt);
                }
                // (a command sent this update - by the machines above or the squad's - goes through the brain command
                // F-83687: its block as the senders build it (Stop F-72181, with Block's handbrake / no throttle / its steer;
                // AlongSide F-94949 with the side and fAlongSideDist; the others F-59773), and every command but Search (7)
                // zeroes the cop's unseen and cone times)
                if std::mem::take(&mut tac.sent) {
                    let blk = cmd_block(tac.cmd, tac.hb, tac.block_steer, tac.side, along);
                    brain_command(&mut tac.brain, tac.cmd as u32, Some(&blk), 0);
                    if tac.cmd != 7 {
                        let e = self.eyes.entry(c.uid).or_default();
                        (e.unseen, e.cone_t) = (tac.brain.unseen, tac.brain.cone_t);
                    }
                }
                if tac.st != st {
                    log::info!("police: cruiser {} {:?} -> {:?} ({:.0} m)", c.uid, st, tac.st, d);
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
            // (a unit's chase is its patrol route's next point - `patrol`)
            let unit = c.vinyl_name.starts_with("police_unit:");
            let Some(ai) = c.ai.as_mut() else { continue };
            if ai.brain.0.is_some() || (ai.chase.is_some() && !unit) || ai.ovr.is_some() {
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
        // (test hook MCLA_POLICE_TRACE: every tactic cop each police frame - its tactic state, its brain's command,
        // place, speed and the overrides on its driver)
        if std::env::var_os("MCLA_POLICE_TRACE").is_some() {
            for c in vehicles.cars.iter().filter(|c| c.cop && self.tacs.contains_key(&c.uid)) {
                let tc = &self.tacs[&c.uid];
                let o = c.ai.as_ref().and_then(|a| a.ovr);
                log::info!("ptrace {:.4} {:?} {} {:?} cmd {} brain {} pos {:.3} {:.3} {:.3} v {:.3} ovr {:?}", self.time, self.heat, c.uid, tc.st, tc.cmd, tc.brain.cmd,
                    c.state.pos.x, c.state.pos.y, c.state.pos.z, c.state.vel.length(), o.map(|o| (o.throttle, o.brake, o.handbrake, o.steer)));
            }
        }
        // the patrol cops: the three units on their patrol paths (F-24233) - no other cruiser is made (the player-load
        // script's three are all the city has)
        if !self.goons {
            let mut busy: Vec<u32> = self.cops.clone();
            busy.extend(self.dsp.as_ref().map(|d| d.cop));
            self.patrol(gpu, world, vehicles, &tun, &busy, dt);
        }
        self.tun = tun;
        self.update_glows(gpu, vehicles);
        event
    }

    /// The tactic's seen update (F-74492, vt+16): +128 takes last frame's +124, then +124 grows and +132 zeroes when the
    /// target is seen, else +132 grows and +124 zeroes.
    fn seen_update(&mut self, seen_now: bool, dt: f32) {
        self.t_seen_prev = self.t_seen;
        if seen_now {
            self.t_seen += dt;
            self.t_unseen = 0.0;
        } else {
            self.t_unseen += dt;
            self.t_seen = 0.0;
        }
    }

    /// The tactic's arrest timer +108 (F-29906) and its test (F-36806): set past fArrestTime at once for a wiped-out
    /// biker with a cop of the tactic within fBikerWipeoutArrestDist; else, with a cop of the tactic within the
    /// tactic's radius + 1 m (F-43276), it grows while the target is under fArrestTgtThresholdSpd and seen this frame
    /// (+128 < +124) and holds otherwise; out of range it is zeroed only once the target has been unseen over 0.5 s.
    /// The test (F-36806) reads last frame's values, before this frame's step: +108 > fArrestTime, +140 > 5, +144 > 5;
    /// not while the Pursuit's Delay runs, whose end zeroes all three (F-25831), nor in the Search (target lost).
    #[allow(clippy::too_many_arguments)]
    fn arrest_step(&mut self, a: ArrestIn, radius: f32, at: f32, bike_r: f32, still_d2: f32, dt: f32) -> bool {
        let _ = (radius, bike_r);
        // (the tactic's update runs the test before F-63286 steps the timers: last frame's values; no test while the
        // Delay runs or a state is asked (F-68032's gate), nor with the target lost (F-77220))
        let fire = !a.delay && !a.lost && (self.arrest > at || self.t_burn > 5.0 || self.t_still > 5.0);
        if a.leaving {
            // (the Pursuit's Delay exit, F-25831)
            self.arrest = 0.0;
            self.t_burn = 0.0;
            self.t_still = 0.0;
        }
        self.arrest_delay = a.delay;
        // (F-29906, behind vt+64 F-20593: not with the target lost; a damaged-out target first, with no distance test; then
        // the downed biker seen by a cop within fBikerWipeoutArrestDist; then a cop seeing it within the radius + 1
        // (F-43276, a line of sight): grows while the target is seen this frame and slow, else holds; with no such cop
        // zeroed once unseen over 0.5 s)
        if a.lost {
        } else if a.wrecked {
            self.arrest = at + 1.0;
        } else if a.bike_down && a.seer_bike {
            self.arrest = at + 1.0;
        } else if a.seer_r1 {
            if self.t_seen_prev < self.t_seen && a.slow {
                self.arrest += dt;
            }
        } else if self.t_unseen > 0.5 {
            self.arrest = 0.0;
        }
        // (the common tactic update F-63286, after the timer: with a tactic cop strictly within the radius - F-29443, not
        // damaged out - and not in the Search (vt+40) +140 counts while the target's burnout charge is up (F-25339) and
        // +144 while it is barely moving (from last frame's place less than (fArrestTgtThresholdSpd x 0.447) x dt, only
        // the left side squared, as coded); without one both are 0; then +48 = the target's place, while the first cop
        // has a brain with a target)
        if a.cop_in && !a.search {
            self.t_burn = if a.burning { self.t_burn + dt } else { 0.0 };
            let still = self.prev_pos.map_or(false, |q| arrest_d2(q, a.pos) < still_d2);
            self.t_still = if still { self.t_still + dt } else { 0.0 };
        } else {
            self.t_burn = 0.0;
            self.t_still = 0.0;
        }
        if a.refresh {
            self.prev_pos = Some(a.pos);
        }
        // (F-36806: +108 > fArrestTime, or either timer over 5 s - a literal)
        fire
    }

    /// The tactic's arrest state zeroed (its reset and constructor: +104 .. +144 and +48).
    fn reset_arrest(&mut self) {
        self.arrest = 0.0;
        self.t_burn = 0.0;
        self.t_still = 0.0;
        self.prev_pos = Some(Vec3::ZERO);
        self.t_seen = 0.0;
        self.t_seen_prev = 0.0;
        self.t_unseen = 0.0;
    }

    /// The fine (F-83946, the routine that fills the arrest form): the highest speed band's
    /// `iLawFines` entry, plus the entry of every other offence (bit n of the original's word
    /// = entry n; a hard hit on a cruiser sets 0x20 and 0x40, the second one is taken back off;
    /// bit 8, the stop sign, is never set); 0 with the NO FINES cheat;
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
        if self.no_charges {
            return 0.0;
        }
        let t = |i: usize| self.tun.array_at("iLawFines", i);
        let mut f = 0i32;
        if self.speed_top > 0 {
            f += t(self.speed_top as usize - 1);
        }
        // (a police car: a soft hit is entry 6; a hard one has 0x20 and 0x40, entry 5 + entry 6 with entry 6 taken back)
        if self.offences & LAW_POLICE_HARD != 0 {
            f += t(5);
        } else if self.offences & LAW_POLICE_VEHICLE != 0 {
            f += t(6);
        }
        for (flag, entry) in [(LAW_PROP, 4), (LAW_VEHICLE, 7), (LAW_RED_LIGHT, 9), (LAW_BURNOUT, 11), (LAW_WRONG_WAY, 13), (LAW_BLOCK_TRAFFIC, 14), (LAW_ROOF_JUMP, 12), (LAW_TWO_WHEEL, 10), (LAW_OFF_ROAD, 3)] {
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

    /// A tactic's end that leaves the player's law record as it is (F-41497 / the tactic's vt+12(0)): the tactic on an
    /// AI racer, which has no record.
    /// The tactic's end, its vt+12 F-67316 with the caller's flag: 1 from the silent end F-41497 and the manager's result
    /// step F-22530 (the escape / cop-gone end), 0 where a new tactic follows (the race retarget in F-22530, the hit-cop
    /// steal in F-52240, StartTactic F-33550). The port never ends a tactic on an arrest (result 1), so `arrested` is false
    /// at every call; the stream unlock F-93264 is not needed (the port streams no racer out).
    fn end_tactic(&mut self, flag: bool) {
        let racing = self.in_race;
        for &u in &self.cops {
            if let Some(e) = tactic_end(1, flag as u32, 0, racing, true) {
                if e.brain_reset {
                    self.lost_ok = true;
                }
                if e.lights_off {
                    self.lamps_off.push(u);
                }
            }
        }
        self.heat = Heat::None;
        self.target = None;
        self.flee_hold = 0.0;
        self.reset_arrest();
        self.lost = 0.0;
        self.search = 0.0;
        self.srch.clear();
        self.kind = TacKind::None;
        self.cops.clear();
        self.eyes.clear();
        self.tacs.clear();
        self.sq = Squad::default();
    }

    fn clear(&mut self) {
        self.heat = Heat::None;
        self.target = None;
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
        self.block_active = false;
        self.sec = [0.0; 7];
        // (and F-88677's other fields: the roof-jump byte +44 (mgr +140), the last junction +56 (+152), the last road +60 and
        // its timer +64 - the radio's road and the freeway test read it)
        self.jump_paid = false;
        self.last_junction = None;
        self.rdo_road = None;
        self.rdo_fw_t = 0.0;
        self.fw_road = false;
        self.bullhorn = None;
        self.ticket_hold = None;
        self.ticketing = false;
        self.flee_hold = 0.0;
        self.reset_arrest();
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

    #[test]
    fn offence_gate_compares_before_adding() {
        let (mut t, dt) = (0.0f32, 0.5f32);
        // threshold 1.0: two adds, the bit on the third tick (compare before add)
        assert!(!offence_gate(true, false, &mut t, 1.0, dt));
        assert!(!offence_gate(true, false, &mut t, 1.0, dt));
        assert_eq!(t, 1.0);
        assert!(offence_gate(true, false, &mut t, 1.0, dt));
        assert_eq!(t, 1.0);
        // started: on with the timer untouched, kept when the condition fails; not started: zeroed
        assert!(offence_gate(true, true, &mut t, 1.0, dt));
        assert!(!offence_gate(false, true, &mut t, 1.0, dt));
        assert_eq!(t, 1.0);
        assert!(!offence_gate(false, false, &mut t, 1.0, dt));
        assert_eq!(t, 0.0);
    }

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

    /// F-98766 run offline (work/hookmen2/b2/unit2.py: 22,856 of 22,856 cases with a finite length equal): a sample of
    /// its answers, bits in and out, the fallback (1, -1) under 1e-12.
    #[test]
    fn unit2_matches_the_original() {
        let f = |x: u32| f32::from_bits(x);
        for (x, y, ox, oy) in [
            (0u32, 0u32, 0x3f800000u32, 0xbf800000u32), (0x33d6bf95, 0, 0x3f800000, 0xbf800000), (0x353be7a2, 0x353be7a2, 0x3f800000, 0xbf800000),
            (0x358637bd, 0, 0x3f800000, 0), (0xb58637bd, 0x358637bd, 0xbf3504f4, 0x3f3504f4), (0x40400000, 0x40800000, 0x3f19999a, 0x3f4ccccd),
            (0xc25f8000, 0x4151999a, 0xbf793dd3, 0x3e69bd97), (0x45848de2, 0x4215fde2, 0x3f7ffd70, 0x3c10d52e), (0x454121fa, 0xc5cb505d, 0x3edba9a6, 0xbf673e11),
            (0xc509d30a, 0xb6c91ed3, 0xbf800000, 0xb13ac8c0), (0x34f0487b, 0xb7175852, 0x3d4af841, 0xbf7faf7d), (0x3f088083, 0xc2521c56, 0x3c264e64, 0xbf7ffca0),
        ] {
            let o = super::unit2(glam::Vec2::new(f(x), f(y)), glam::Vec2::new(1.0, -1.0));
            assert_eq!((o.x.to_bits(), o.y.to_bits()), (ox, oy), "{x:08x} {y:08x}");
        }
    }

    /// Dispatch to Location's machine against the original's tactic object run offline (tools/dispatch_fsm_reference.py:
    /// ctor, reset, engage, 900 updates of F-87106 in 48 cases of speech windows and arrival frames; there all 43,200
    /// updates equal): every update where the state or the state asked for changed, and the last, with the state's time
    /// bit for bit.
    #[test]
    fn dispatch_machine_matches_the_original() {
        let reference = include_str!("testdata/dispatch_fsm_ref.txt");
        let (mut checked, mut case) = (0, None::<(i64, i64, i64)>);
        let mut d = None::<Dispatch>;
        let mut k = 0i64;
        for line in reference.lines() {
            let v: Vec<&str> = line.split_whitespace().collect();
            if v[0] == "CASE" {
                case = Some((v[1].parse().unwrap(), v[2].parse().unwrap(), v[3].parse().unwrap()));
                d = Some(Dispatch { cop: 1, st: 0, t: 0.0, loc: Vec3::ZERO, call: String::new(), crumbs: Vec::new(), crumb_t: 1.0, seen_t: 0.0,
                    trail: false, routed: false, arrived: false, near_t: 0.0, go_t: 0.0, next: None, brain: BrainCmd::default() });
                k = 0;
                continue;
            }
            let (a, b, c) = case.unwrap();
            let d = d.as_mut().unwrap();
            let v = if v[0] == "END" { &v[1..] } else { &v[..] };
            let want: i64 = v[0].parse().unwrap();
            // (the update's order in Police::update: enter, the tests, the time)
            while k <= want {
                dispatch_enter(d);
                d.arrived = k >= c;
                dispatch_ask(d, 3.0, k >= a && k < b);
                d.t += 1.0 / 60.0;
                k += 1;
            }
            let got = (d.st as i32, d.next.map_or(-1, |n| n as i32), d.t.to_bits());
            let exp = (v[1].parse().unwrap(), v[2].parse().unwrap(), u32::from_str_radix(v[3], 16).unwrap());
            assert_eq!(got, exp, "case {a} {b} {c} update {want}");
            checked += 1;
        }
        assert!(checked > 100, "{checked}");
    }

    /// The tactic radio's chatter step against the original's F-26639 run offline (tools/radio_chatter_reference.py: the
    /// tactic object in guest memory, the world's answers (road, freeway name, contact entries, vicinity, call signs, the say)
    /// stubbed as the case's inputs, the rest of the function and the scenario classes unchanged; 4,000 cases of 60 frames,
    /// 240,000 frames equal, this file a covering subset): the line said, its sender and receiver, the countdown, the law
    /// record's road and timer, the nitrous tries, the far byte and the generator, bit for bit.
    #[test]
    fn radio_chatter_matches_the_original() {
        let r = rdo_replay(include_str!("testdata/radio_chatter_ref.txt"));
        assert!(r.len() > 1000, "{}", r.len());
        for (ln, want, got) in &r {
            assert_eq!(got, want, "line {ln}");
        }
    }

    /// Replays the tool's file through `chatter_step`: per F line, the original's outputs and the port's.
    fn rdo_replay(text: &str) -> Vec<(usize, String, String)> {
        let h = |s: &str| u32::from_str_radix(s, 16).unwrap();
        let f = |s: &str| f32::from_bits(h(s));
        let mut out = Vec::new();
        let (mut n, mut fw, mut min_t, mut max_t) = (0usize, 0u32, 0f32, 0f32);
        let mut st = RdoState { t: 0.0, road: None, fw_t: 0.0, nitro: 0, far: false, rng: (0, 0) };
        for (ln, l) in text.lines().enumerate() {
            let v: Vec<&str> = l.split_whitespace().collect();
            match v.first().copied() {
                Some("CASE") => {
                    n = v[1].parse().unwrap();
                    fw = h(v[2]);
                    min_t = f(v[3]);
                    max_t = f(v[4]);
                    let r = u64::from_str_radix(v[11], 16).unwrap();
                    st = RdoState { t: f(v[6]), road: v[7].parse().ok(), fw_t: f(v[8]), nitro: v[9].parse().unwrap(), far: v[10] == "1", rng: (r as u32, (r >> 32) as u32) };
                }
                Some("F") => {
                    let hit: Vec<Option<f32>> = v[16..20].iter().take(n).map(|s| if *s == "-" || *s == "n" { None } else { Some(f(s)) }).collect();
                    let chain: u32 = v[9].parse().unwrap();
                    let is_fw = |r: usize| fw >> r & 1 != 0;
                    let i = RdoIn {
                        dt: f(v[1]),
                        on: v[2] == "1",
                        busy: v[3] == "1",
                        n,
                        target: v[4] == "1",
                        is_player: v[5] == "1",
                        rec: v[6] == "1",
                        road: v[7].parse().ok(),
                        is_fw: &is_fw,
                        hard: v[8] == "1",
                        hit: &hit,
                        nitro_burning: chain == 3 && v[10] != "0",
                        speed: if chain >= 2 && v[4] == "1" { f(v[11]) } else { 0.0 },
                        in_race: v[12] == "1",
                        mgr80: v[13].parse().unwrap(),
                        within_vic: v[14] == "1",
                        within_10: v[15] == "1",
                        min_t,
                        max_t,
                    };
                    let call = chatter_step(&mut st, &i);
                    let got = format!(
                        "{} {} {} {:x} {:x} {} {} {} {:016x}",
                        call.map_or("-", |c| c.0),
                        call.map_or("-".to_string(), |c| c.1.to_string()),
                        call.map_or("-".to_string(), |c| c.2.map_or("D".to_string(), |r| r.to_string())),
                        st.t.to_bits(),
                        st.fw_t.to_bits(),
                        st.road.map_or("-".to_string(), |r| r.to_string()),
                        st.nitro,
                        st.far as u8,
                        (st.rng.1 as u64) << 32 | st.rng.0 as u64
                    );
                    out.push((ln + 1, v[21..].join(" "), got));
                }
                _ => {}
            }
        }
        out
    }

    /// The Arrow Trap's CatchUp / Halt machine against the original's tactic object run offline (tools/squad_fsm_reference.py:
    /// ctor, reset, begin, 300 updates of F-76774 in 30 cases of formation sends, a cop within, the target moving / fast;
    /// there all 9,000 updates equal): every update where the state or the state asked for changed, and the last, with
    /// CatchUp's lock bit for bit.
    #[test]
    fn squad_machine_matches_the_original() {
        let reference = include_str!("testdata/squad_fsm_ref.txt");
        let (mut checked, mut flags, mut k) = (0, Vec::new(), 0usize);
        let mut sq = Squad::default();
        for line in reference.lines() {
            let v: Vec<&str> = line.split_whitespace().collect();
            if v[0] == "CASE" {
                flags = v[1].chars().map(|c| c.to_digit(16).unwrap()).collect();
                sq = Squad::default();
                k = 0;
                continue;
            }
            let v = if v[0] == "END" { &v[1..] } else { &v[..] };
            let want: usize = v[0].parse().unwrap();
            while k <= want {
                let f = |b: u32| flags[k] & b != 0;
                let send = f(8);
                squad_machine(&mut sq, &mut (), true, f(4), f(2), f(1), 1.0 / 60.0, &mut |sq: &mut Squad, _: &mut (), d: f32, _| {
                    if send || d == 0.0 {
                        sq.slots = [Some(0), Some(1), Some(2)];
                    }
                    send
                }, &mut |_: &mut Squad, _: &mut ()| {});
                k += 1;
            }
            let st = if sq.halt { 2 } else { 0 };
            let got = (st, sq.next.map_or(-1, |h| if h { 2 } else { 0 }), if st == 0 { sq.lock.to_bits() } else { 0 });
            let exp = (v[1].parse().unwrap(), v[2].parse().unwrap(), u32::from_str_radix(v[3], 16).unwrap());
            assert_eq!(got, exp, "update {want}");
            checked += 1;
        }
        assert!(checked > 100, "{checked}");
    }

    /// The Double Pursuit's machine (AlongSide, no Halt) against the original's tactic object run offline by
    /// tools/pursuit_dbl_fsm_reference.py (ctor F-74544, reset F-92837, begin F-39789, update F-40655, AlongSide F-86874 /
    /// F-41674): the same flags, state, asked state and AlongSide's lock bits equal at every recorded update.
    #[test]
    fn double_pursuit_machine_matches_the_original() {
        let reference = include_str!("testdata/pursuit_dbl_fsm_ref.txt");
        let (mut checked, mut flags, mut k) = (0, Vec::new(), 0usize);
        let mut sq = Squad::default();
        for line in reference.lines() {
            let v: Vec<&str> = line.split_whitespace().collect();
            if v[0] == "CASE" {
                flags = v[1].chars().map(|c| c.to_digit(16).unwrap()).collect();
                sq = Squad::default();
                k = 0;
                continue;
            }
            let v = if v[0] == "END" { &v[1..] } else { &v[..] };
            let want: usize = v[0].parse().unwrap();
            while k <= want {
                let f = |b: u32| flags[k] & b != 0;
                let send = f(8);
                squad_machine(&mut sq, &mut (), false, f(4), f(2), f(1), 1.0 / 60.0, &mut |sq: &mut Squad, _: &mut (), d: f32, _| {
                    if send || d == 0.0 {
                        sq.slots = [Some(0), Some(1), None];
                    }
                    send
                }, &mut |_: &mut Squad, _: &mut ()| {});
                k += 1;
            }
            let st = if sq.halt { 2 } else { 0 };
            let got = (st, sq.next.map_or(-1, |h| if h { 2 } else { 0 }), if st == 0 { sq.lock.to_bits() } else { 0 });
            let exp = (v[1].parse().unwrap(), v[2].parse().unwrap(), u32::from_str_radix(v[3], 16).unwrap());
            assert_eq!(got, exp, "update {want}");
            checked += 1;
        }
        assert!(checked > 500, "{checked}");
    }

    /// The Search clock test against the original's F-73289 run on all 2^32 clock patterns by tools/search_clock_reference.py:
    /// every pattern in the true ranges and their neighbours, and every 4,099th pattern, give the original's answer.
    #[test]
    fn search_clock_matches_the_original() {
        let reference = include_str!("testdata/search_clock_ref.txt");
        let mut ranges = Vec::new();
        let mut count = 0u64;
        for l in reference.lines() {
            if let Some(r) = l.strip_prefix("true ") {
                let (a, b) = r.split_once('-').unwrap();
                ranges.push((u32::from_str_radix(a, 16).unwrap(), u32::from_str_radix(b, 16).unwrap()));
            } else if let Some(n) = l.strip_prefix("count ") {
                count = n.parse().unwrap();
            }
        }
        assert_eq!(ranges.iter().map(|&(a, b)| (b - a) as u64 + 1).sum::<u64>(), count);
        let orig = |b: u32| ranges.iter().any(|&(lo, hi)| (lo..=hi).contains(&b));
        let mut probe: Vec<u32> = (0..=u32::MAX / 4099).map(|i| i * 4099).collect();
        for &(lo, hi) in &ranges {
            probe.extend([lo.wrapping_sub(1), lo, lo.wrapping_add(1), hi.wrapping_sub(1), hi, hi.wrapping_add(1)]);
        }
        probe.extend([0x8000_0000, 0xFF80_0000, 0x7FC0_0000, 0xFFC0_0000, 0x7F80_0001]);
        for b in probe {
            assert_eq!(search_clock_runs(f32::from_bits(b)), orig(b), "{b:08x}");
        }
    }

    /// tools/police_getters_reference.py's lines of one kind, as words (hex; a leading '-' negates)
    fn getters_ref(kind: &str) -> Vec<Vec<u32>> {
        let word = |x: &str| {
            let v = i64::from_str_radix(x.trim_start_matches('-'), 16).unwrap();
            (if x.starts_with('-') { -v } else { v }) as u32
        };
        include_str!("testdata/police_getters_ref.txt")
            .lines()
            .filter(|l| l.split_whitespace().next() == Some(kind))
            .map(|l| l.split_whitespace().skip(1).map(word).collect())
            .collect()
    }

    #[test]
    fn running_arm_matches_the_original() {
        // (F-75755 / F-93041 run offline: count 0, 1, 4 x index -1 .. 5 x the timer around 6.5 and 8.5, inf, NaN; arm k is
        // (1, 2, -1) with k << 20 added to each word)
        let mut n = 0;
        for (kind, f) in [("G", arm_green as fn(f32, i32, &[Vec3]) -> Vec3), ("Y", arm_yellow)] {
            for v in getters_ref(kind) {
                let arms: Vec<Vec3> = (0..v[0])
                    .map(|k| Vec3::new(f32::from_bits(0x3f800000 + (k << 20)), f32::from_bits(0x40000000 + (k << 20)), f32::from_bits(0xbf800000 + (k << 20))))
                    .collect();
                let a = f(f32::from_bits(v[2]), v[1] as i32, &arms);
                assert_eq!([a.x.to_bits(), a.y.to_bits(), a.z.to_bits()], [v[3], v[4], v[5]], "{kind} {v:x?}");
                n += 1;
            }
        }
        assert_eq!(n, 330);
        // (the callers' order: green, the yellow when green is the zero vector - one arm while green or yellow)
        let arms = [Vec3::X, Vec3::Z];
        assert_eq!(running_arm(1.0, 1, &arms), Vec3::Z);
        assert_eq!(running_arm(7.0, 0, &arms), Vec3::X);
        assert_eq!(running_arm(8.5, 0, &arms), Vec3::ZERO);
    }

    #[test]
    fn default_final_speed_matches_the_original() {
        // (F-85634 run offline: no target / no racer / no car / the car's speed around f32(fMovingSpeed x 0.447), 30, NaN)
        let v = getters_ref("S");
        for c in &v {
            let sp = (c[1] == 3).then(|| f32::from_bits(c[2]));
            assert_eq!(default_final_speed(sp, f32::from_bits(c[0])).to_bits(), c[3], "{c:x?}");
        }
        assert_eq!(v.len(), 18);
    }

    /// The say's choice of F-66952 / F-14569 (docs/reference/hookmen/law_reports.md) on chosen draws: the default class's
    /// first draw under 0.25 makes it the name, else the second under 0.2; failing both, the bits in F-14569's order.
    /// tools/lost_flag_reference.py: the original's track set F-79519 and common update F-63286 run offline over the four tactic
    /// classes (1,056 cases, 900 updates each): +136 and every cop's +773 equal the port's `track_step` on every recorded
    /// update (a set: +136 = the value, +773 cleared on every cop).
    #[test]
    fn lost_flag_matches_the_original() {
        let h = |t: &str| u32::from_str_radix(t, 16).unwrap();
        let (mut cases, mut rows) = (0, 0);
        let mut it = include_str!("testdata/lost_flag_ref.txt").lines().peekable();
        while let Some(head) = it.next() {
            let v: Vec<&str> = head.split(' ').collect();
            assert_eq!(v[0], "CASE");
            let (cls, p29, nc, dt) = (v[1], v[2] == "1", v[3].parse::<u32>().unwrap(), f32::from_bits(h(v[4])));
            let sets: Vec<(usize, f32)> = v[5].split(',').map(|kv| {
                let (k, b) = kv.split_once(':').unwrap();
                (k.parse().unwrap(), f32::from_bits(h(b)))
            }).collect();
            let answers = p29 && cls != "3";
            let mut rec = std::collections::BTreeMap::new();
            while let Some(l) = it.peek().filter(|l| !l.starts_with("CASE")) {
                let w: Vec<&str> = l.split(' ').collect();
                rec.insert(w[0].parse::<usize>().unwrap(), (h(w[1]), w[2].parse::<u32>().unwrap()));
                it.next();
            }
            let (mut t, mut lost) = (0.0f32, true);
            for k in 0..900 {
                for &(sk, sv) in &sets {
                    if sk == k {
                        t = sv;
                        lost = false;
                    }
                }
                track_step(&mut t, &mut lost, dt, answers);
                if let Some(&(tb, mask)) = rec.get(&k) {
                    assert_eq!((t.to_bits(), if lost { (1 << nc) - 1 } else { 0 }), (tb, mask), "{head} update {k}");
                    rows += 1;
                }
            }
            cases += 1;
        }
        assert_eq!((cases, rows), (1056, 12549 - 1056));
    }

    /// tools/zone_pick_reference.py: the original's EvaluateCopZones body F-88125 run offline (3,120 cases: the dev override,
    /// 0 - 5 police, 0 - 11 zones, no race / a race (its +8 word 1 or 0), laps 0 - 8 and -1, random generator states): the
    /// zone, the lap, the armed state, the underground call and the generator after equal `zone_pick`.
    #[test]
    fn zone_pick_matches_the_original() {
        let g = |t: &str| u32::from_str_radix(&t[1..], 16).unwrap();
        let mut n = 0;
        for l in include_str!("testdata/zone_pick_ref.txt").lines() {
            let (lhs, rhs) = l.split_once(" -> ").unwrap();
            let v: Vec<&str> = lhs.split(' ').collect();
            let u = |i: usize| v[i].parse::<u32>().unwrap();
            let ovr = (u(0) != 0).then(|| u(1) as i32);
            let laps = (u(4) != 0).then(|| u(5) as i32);
            let mut rng = (g(v[6]), g(v[7]));
            let got = zone_pick(&mut rng, ovr, u(2) as i32, u(3) as u16, laps);
            let (zone, lap) = got.unwrap_or((-1, -1));
            let armed = matches!(got, Some((_, 0))) as u32;
            let want = format!("{zone} {lap} {armed} 0 p00000000 {} g{:08x} g{:08x}", got.is_some() as u32, rng.0, rng.1);
            assert_eq!(want, rhs, "{lhs}");
            n += 1;
        }
        assert_eq!(n, 3120);
    }

    /// tools/zone_setoff_reference.py: the original's zone machine F-71212 in state 1 run offline (4,000 cases: the type 0 - 3
    /// and 7, the percentage, fRaceTgtAiDist (negative, 0, NaN, huge), the radius, 0 - 5 racers round the zone, the AI
    /// list absent / 0 - 7 entries with nulls and equal distances, the scripted racer in the racer list or not, the
    /// generator's state): the state, the target, the timer, the requests and the generator after equal `zone_set_off`;
    /// and 2,000 ticks of states 2 / 3 (the timer, dt, a target or none, damaged out or not, the delay): the state, the
    /// timer, the damaged-out questions and the Arrow Trap's start equal `zone_tick`.
    #[test]
    fn zone_set_off_matches_the_original() {
        let h = |t: &str| u32::from_str_radix(&t[1..], 16).unwrap();
        let mut ticks = 0;
        let pt = |t: &str| {
            let w: Vec<f32> = t.split(':').map(|x| f32::from_bits(u32::from_str_radix(x, 16).unwrap())).collect();
            Vec3::new(w[0], w[1], w[2])
        };
        let mut n = 0;
        for l in include_str!("testdata/zone_setoff_ref.txt").lines() {
            let (lhs, rhs) = l.split_once(" -> ").unwrap();
            if let Some(t) = lhs.strip_prefix("T ") {
                let v: Vec<&str> = t.split(' ').collect();
                let (st, tg, wr) = (v[0].parse::<u8>().unwrap(), v[1] == "1", v[4] == "1");
                let mut asked = 0;
                let (st2, tm, start) = zone_tick(st, tg, f32::from_bits(h(v[2])), f32::from_bits(h(v[3])), f32::from_bits(h(v[5])), || {
                    asked += 1;
                    wr
                });
                // (the start: StartTactic type 4, r6 1, r7 0, on F-30913's tactic built with f1 0.0 on the target)
                let want = format!("{st2} f{:08x} {asked} t00000000 s{}", tm.to_bits(), if start { "1041" } else { "0" });
                assert_eq!(want, rhs, "{lhs}");
                ticks += 1;
                continue;
            }
            let (a, b) = lhs.split_once(" | ").unwrap();
            let v: Vec<&str> = a.split(' ').collect();
            let (kind, prob, dist, r, zone) = (v[0].parse::<u32>().unwrap(), v[1].parse::<i32>().unwrap(), f32::from_bits(h(v[2])),
                                               f32::from_bits(h(v[3])), pt(v[4]));
            let racers: Vec<Vec3> = v[5..].iter().filter(|t| **t != ".").map(|t| pt(t)).collect();
            let w: Vec<&str> = b.split(' ').collect();
            let k = w.len();
            let ai: Option<Vec<Option<Vec3>>> = (w[0] != "null").then(|| w[1..k - 4].iter().map(|t| (*t != "n").then(|| pt(t))).collect());
            let mask = w[k - 3][1..].parse::<u32>().unwrap();
            let scripted = w[k - 4][1..].parse::<usize>().ok().filter(|&i| mask >> i & 1 == 1);
            let mut rng = (h(w[k - 2]), h(w[k - 1]));
            let got = zone_set_off(&mut rng, kind, prob, dist, zone, r, &racers, scripted, ai.as_deref());
            let tgt = match got.and_then(|g| g.1) {
                None => "0".to_string(),
                Some(ZoneTgt::Racer(i)) => format!("r{i}"),
                Some(ZoneTgt::Ai(j)) => format!("a{j}"),
            };
            let want = format!("{} {tgt} f{} {} g{:08x} g{:08x}", if got.is_some() { 2 } else { 1 },
                               if got.is_some() { "40b00000" } else { "7f7f7f7f" }, got.map_or(0, |g| g.0), rng.0, rng.1);
            assert_eq!(want, rhs, "{lhs}");
            n += 1;
        }
        assert_eq!((n, ticks), (4000, 2000));
    }

    /// tools/tactic_end_reference.py: the original's tactic end F-67316 run offline (2,506 cases: the active byte, the flag word,
    /// the result, a race current or not, the global present or not, 0 - 4 cop entries null / car / car with a brain, the
    /// count also 0 and -1): the return, the global's byte, the callee events in order and every brain's +773 / +408 equal
    /// what `tactic_end` gives.
    #[test]
    fn tactic_end_matches_the_original() {
        let mut n = 0;
        for l in include_str!("testdata/tactic_end_ref.txt").lines() {
            let (lhs, rhs) = l.split_once(" -> ").unwrap();
            let v: Vec<&str> = lhs.split(' ').collect();
            let num = |i: usize| v[i].parse::<u64>().unwrap();
            let (act, flag, res, race, glob, cnt) = (num(0) as u8, num(1) as u32, num(2) as u32, num(3) != 0, num(4) != 0, num(5) as u32 as i32);
            let kinds = &v[6..];
            let (mut ev, mut brains) = (Vec::new(), Vec::new());
            let mut ret = 0;
            if let Some(_) = tactic_end(act, flag, res, race, false) {
                ret = 1;
                for (i, k) in kinds.iter().enumerate().take(cnt.max(0) as usize) {
                    if *k == "n" {
                        continue;
                    }
                    let e = tactic_end(act, flag, res, race, *k == "b").unwrap();
                    if e.brain_reset {
                        ev.push(format!("C:{i}:x0"));
                    }
                    if e.siren_stop {
                        ev.push(format!("S:{i}"));
                    }
                    if e.lights_off {
                        ev.push(format!("L:{i}:x0"));
                    }
                    if e.unlock {
                        ev.push(format!("U:{i}"));
                    }
                }
                ev.push("V56:T".into());
                ev.push("V4:T".into());
            }
            for (i, k) in kinds.iter().enumerate() {
                let reset = ret == 1 && (i as i32) < cnt && *k == "b";
                brains.push(match *k {
                    "b" if reset => "k1:x00000000".to_string(),
                    "b" => "k51:x77777777".to_string(),
                    _ => "-".to_string(),
                });
            }
            let gb = if ret == 1 && glob { 0 } else { 0x5A };
            let want = format!("r{ret} g{gb} {} | {}", if ev.is_empty() { ".".into() } else { ev.join(" ") }, if brains.is_empty() { ".".into() } else { brains.join(" ") });
            assert_eq!(want, rhs, "{lhs}");
            n += 1;
        }
        assert_eq!(n, 2506);
    }

    #[test]
    fn report_line_follows_the_original() {
        let run = |default: bool, bits: u32, d: &[f32]| {
            let mut it = d.iter().copied();
            let mut n = 0;
            let line = report_line("REPORT_VIOLATION_PURSUIT", default, bits, || {
                n += 1;
                it.next().unwrap()
            });
            (line, n)
        };
        assert_eq!(run(true, 0x40, &[0.1, 0.9]), ("REPORT_VIOLATION_PURSUIT", 2));
        assert_eq!(run(true, 0x40, &[0.3, 0.19]), ("REPORT_VIOLATION_PURSUIT", 2));
        assert_eq!(run(true, 0x40, &[0.3, 0.2]), ("REPORT_HIT_COP_CAR_SOFT", 2));
        assert_eq!(run(true, 0x60, &[0.3, 0.5]), ("REPORT_HIT_COP_CAR_HARD", 2));
        assert_eq!(run(true, 0x2201, &[0.3, 0.5]), ("REPORT_VIOLATION_REDLIGHTS", 2));
        assert_eq!(run(true, 0x2009, &[0.3, 0.5]), ("REPORT_VIOLATION_RECKLESS_SIDEWALK", 2));
        assert_eq!(run(true, 0x2001, &[0.3, 0.5]), ("REPORT_VIOLATION_RECKLESS_WRONGSIDE", 2));
        assert_eq!(run(true, 0x4, &[0.3, 0.5]), ("REPORT_VIOLATION_SPEEDING", 2));
        assert_eq!(run(true, 0x1400, &[0.3, 0.5]), ("REPORT_VIOLATION_PURSUIT", 2));
        assert_eq!(run(false, 0x40, &[0.9999]), ("REPORT_VIOLATION_PURSUIT", 1));
    }

    /// Every case of tools/report_say_reference.py (the original's F-66952 / F-14569 run offline, the request stubbed): the
    /// line asked (or none) and the generator's state after, so the draws are the same in number and in value.
    #[test]
    fn report_line_matches_the_original() {
        let h = |t: &str| u32::from_str_radix(&t[1..], 16).unwrap();
        let mut n = 0;
        for l in include_str!("testdata/report_say_ref.txt").lines() {
            let (lhs, rhs) = l.split_once(" -> ").unwrap();
            let a: Vec<&str> = lhs.split(' ').collect();
            let b: Vec<&str> = rhs.split(' ').collect();
            let mut g = crate::vehicles::parked::Mwc { lo: h(a[9]), carry: h(a[10]) };
            let bits = h(a[2]);
            let name = match a[1] { "2" => Some("REPORT_VIOLATION_PURSUIT"), _ => None };
            let line = if a[0] == "0" {
                Some(report_line(name.unwrap(), true, bits, || g.float()))
            } else {
                report_say(name, f32::from_bits(h(a[3])), bits, || g.float())
            };
            assert_eq!(line.unwrap_or("-"), b[1], "{l}");
            assert_eq!(b[0] == "1", line.is_some(), "{l}");
            assert_eq!((g.lo, g.carry), (h(b[6]), h(b[7])), "{l}");
            n += 1;
        }
        assert_eq!(n, 2100);
    }

    /// StateTicket's clock against recording t1 (docs/reference/hookmen/pullover_recording.md): entered in update 451
    /// (the exit's time = dt 0x3D088889), the result set in update 504; the state time at each exit grows by the update's
    /// dt (503: 0x3FE2221F, 504: 0x3FE66663). The car was not damaged out (+68 = 1: limit fArrestStateTime 1.75).
    #[test]
    fn ticket_clock_matches_the_recording() {
        let dt = f32::from_bits(0x3D08_8889);
        let mut hold = (dt, 1.75f32);
        let mut times = vec![(451, hold.0.to_bits())];
        let mut result = None;
        for k in 452..=504 {
            if ticket_tick(&mut hold, dt) && result.is_none() {
                result = Some(k);
            }
            times.push((k, hold.0.to_bits()));
        }
        assert_eq!(result, Some(504));
        assert_eq!(times[0], (451, 0x3D08_8889));
        assert_eq!(times[times.len() - 2], (503, 0x3FE2_221F));
        assert_eq!(times[times.len() - 1], (504, 0x3FE6_6663));
    }

    #[test]
    fn pullover_cap_open_matches_the_original() {
        // (F-98647 on the pull-over's own vtable run offline: +28 x the arrest timer +108 x the bullhorn timer +112 x the FSM
        // state 0 .. 3, Ticket = 3)
        let v = getters_ref("P");
        for c in &v {
            let r = pullover_may_escalate(c[0] == 1, f32::from_bits(c[1]), f32::from_bits(c[2]), c[3] == 3);
            assert_eq!(r, c[4] == 1, "{c:x?}");
        }
        assert_eq!(v.len(), 128);
    }

    #[test]
    fn race_chase_type_matches_the_original() {
        // (F-12315 run offline with the manager set: type 0, 1, 2, 7 x no racer / the player / an AI racer; without the
        // manager nothing is written)
        let v = getters_ref("Z");
        for c in &v {
            if c[0] == 0 {
                assert_eq!((c[3], c[4]), (0x85, 0), "{c:x?}");
                continue;
            }
            let r = match c[2] {
                0 => None,
                1 => Some(true),
                _ => Some(false),
            };
            assert_eq!(race_chase_type(c[1] as u8, r) as u32, c[3], "{c:x?}");
            assert_eq!(c[4] == 1, r.is_some(), "{c:x?}");
        }
        assert_eq!(v.len(), 24);
        assert_eq!(PoliceEval::SH_04B.chase_type, 1);
        assert_eq!(PoliceEval::GATED.chase_type, 0);
    }

    #[test]
    fn patrol_hold_matches_the_original() {
        // (F-30531 run offline: group -1, 0, 3 x path -1, 0, 9 x fPatrolTime 5, 60; the hold +1128 starts at 0.5)
        let v = getters_ref("A");
        for c in &v {
            assert_eq!((c[2], c[3]), (c[0], c[1]), "{c:x?}");
            assert_eq!(patrol_hold(c[0] as i32, 0.5, f32::from_bits(c[5])).to_bits(), c[4], "{c:x?}");
        }
        assert_eq!(v.len(), 18);
    }

    #[test]
    fn add_points_matches_the_original() {
        // (tools/add_points_reference.py: F-19359 run whole offline - the racer list F-45642, the tactic lookup F-59036, the five
        // tactic classes on their own vtables - over 143 tactic set-ups x 15 (score, points) x 2 tunes)
        let mut n = 0;
        for l in include_str!("testdata/add_points_ref.txt").lines() {
            let w: Vec<&str> = l.split_whitespace().skip(1).collect();
            let u = |i: usize| u32::from_str_radix(w[i], 16).unwrap();
            let d = |i: usize| w[i].parse::<i32>().unwrap();
            let (cls, may, b92, arrest, bullhorn, st, radio) = (d(0), d(1) == 1, d(2), f32::from_bits(u(3)), f32::from_bits(u(4)), d(5), d(6) == 1);
            let tac = (cls <= 4).then(|| LawTactic {
                ty: cls,
                may,
                sounded: b92 == 2,
                escalates: match cls {
                    1 => pullover_may_escalate(may, arrest, bullhorn, st == 3),
                    2 => pursuit_may_escalate(st == 0, radio, may, st == 6, st == 4, arrest),
                    3 => tactic_may_escalate(radio, may, st == 2, st == 1, arrest),
                    _ => false,
                },
            });
            let got = add_points(d(9), d(10), tac, d(7), d(8)).unwrap_or(d(9));
            assert_eq!(got, d(11), "{l}");
            n += 1;
        }
        assert_eq!(n, 4290);
    }

    #[test]
    fn go_to_arrived_matches_the_original() {
        // (tools/brain_states_reference.py: F-16378 with HasReachedGoal F-43234 run offline - +580 x the state time x the route
        // (+128 set / no aiv / not done / done) x +576 before 0, 1, 7; the port's arrival is +576 == 1)
        let mut n = 0;
        for l in include_str!("testdata/brain_states_ref.txt").lines().filter(|l| l.starts_with("K ")) {
            let w: Vec<&str> = l.split_whitespace().skip(1).collect();
            let f = |i: usize| f32::from_bits(u32::from_str_radix(w[i], 16).unwrap());
            let (route, before, after) = (w[2].parse::<u32>().unwrap(), w[3].parse::<i32>().unwrap(), w[4].parse::<i32>().unwrap());
            let latched = go_to_arrived(false, f(0), f(1), route == 3);
            assert_eq!(after, if latched { 1 } else { before }, "{l}");
            n += 1;
        }
        assert_eq!(n, 300);
    }

    #[test]
    fn brain_pieces_match_the_original() {
        // (the same run: F-98680 the brain's +576 or 0 with no brain; F-55899 a zero splat - the planner's final heading, as
        // brain::Brain::set_goals sets it; F-97098 Search's reset - count 0, the route flag 0, the ten points cleared - as a new
        // CopSearch starts)
        let ref_ = include_str!("testdata/brain_states_ref.txt");
        let r: Vec<&str> = ref_.lines().filter(|l| l.starts_with("R ")).collect();
        assert_eq!(r, ["R 0 0 0", "R 1 0 0", "R 1 1 1", "R 1 2 2", "R 1 -1 -1"]);
        for l in ref_.lines().filter(|l| l.starts_with("H ")) {
            assert!(l.split_whitespace().skip(2).all(|x| x == "00000000"), "{l}");
        }
        let q = |pat: u32, k: usize| -> u32 {
            let l = ref_.lines().find(|l| l.starts_with(&format!("Q {pat} {k} "))).unwrap();
            u32::from_str_radix(l.split_whitespace().nth(3).unwrap(), 16).unwrap()
        };
        let s = search_start(0, Vec::new(), false, 0.0);
        for pat in 0..2 {
            assert_eq!(q(pat, 84) as usize, s.pts.len());
            assert_eq!((q(pat, 85) >> 24) != 0, s.routed);
            for i in 0..10 {
                // (each record's point +16 + 32 i and its node word +32 + 32 i)
                for k in 0..5 {
                    assert_eq!(q(pat, 4 + 8 * i + k), 0);
                }
            }
        }
    }

    #[test]
    fn damaged_out_matches_the_original() {
        // (tools/police_predicates_reference.py: F-12032 run offline; the port's damage is points / MaxDamage capped at 1.0,
        // as physics::car::CarState's hits store it)
        let mut n = 0;
        for l in include_str!("testdata/police_predicates_ref.txt").lines().filter(|l| l.starts_with("D ")) {
            let v: Vec<u32> = l.split_whitespace().skip(1).map(|x| u32::from_str_radix(x, 16).unwrap()).collect();
            let (max, points) = (f32::from_bits(v[0]), f32::from_bits(v[1]));
            assert_eq!(damaged_out((points / max).min(1.0)), v[2] == 1, "{l}");
            n += 1;
        }
        assert_eq!(n, 60);
    }

    #[test]
    fn needs_teleport_matches_the_original() {
        // (F-14059 run offline: underground x brain none / off-mesh 0, 5, 5 + 1 ulp, 9 x car none / not out / out with its delay;
        // the cars' MaxDamage 400, not out = 200 points)
        let mut n = 0;
        for l in include_str!("testdata/police_predicates_ref.txt").lines().filter(|l| l.starts_with("T ")) {
            let v: Vec<u32> = l.split_whitespace().skip(1).map(|x| u32::from_str_radix(x, 16).unwrap()).collect();
            let off = if v[1] == 1 { f32::from_bits(v[2]) } else { 0.0 };
            let out = v[3] == 2 && damaged_out((400.0f32 / 400.0).min(1.0));
            assert!(v[3] != 1 || !damaged_out(200.0 / 400.0));
            assert_eq!(needs_teleport(v[0] == 1, off, out, f32::from_bits(v[4])), v[5] == 1, "{l}");
            n += 1;
        }
        assert_eq!(n, 60);
    }

    #[test]
    fn moving_target_matches_the_original() {
        // (F-96080 run offline: the target car's speed over f32(fMovingSpeed x 0.447))
        let mut n = 0;
        for l in include_str!("testdata/police_predicates_ref.txt").lines().filter(|l| l.starts_with("M ")) {
            let v: Vec<u32> = l.split_whitespace().skip(1).map(|x| u32::from_str_radix(x, 16).unwrap()).collect();
            assert_eq!(moving(f32::from_bits(v[1]), f32::from_bits(v[0])), v[2] == 1, "{l}");
            n += 1;
        }
        assert_eq!(n, 24);
    }

    #[test]
    fn arrest_hold_strip_is_activity_100() {
        // (F-35540: the manager's +56 - set by F-22530 on the arrest's success frame, cleared by CutSceneDone - is tested after
        // the Search tests and gives 100, which the strip's table F-61846 shows as look 0; the tactic was ended there)
        let mut p = Police::new();
        p.start_tactic(TacKind::Pursuit, vec![], true, 25.0);
        p.heat = Heat::Busted;
        assert_eq!(p.hud_state(), 5);
        p.arrest_posted = true;
        assert_eq!(p.hud_state(), 100);
        p.heat = Heat::None;
        p.arrest_posted = false;
        p.kind = TacKind::None;
        assert_eq!(p.hud_state(), 0);
    }

    #[test]
    fn burn_tests_match_the_original() {
        // tools/burn_tests_reference.py: F-25339 on the special floats and 2,000 random words (and no car / chassis),
        // F-48805 on every byte 0..255 (and no racer / car)
        let (mut a_n, mut b_n) = (0, 0);
        for l in include_str!("testdata/burn_tests_ref.txt").lines() {
            let (a, b) = l.split_once(" | ").unwrap();
            let v: Vec<&str> = a.split_whitespace().collect();
            let (w, null): (u32, u32) = (v[1].parse().unwrap(), v[2].parse().unwrap());
            let got = if v[0] == "charge" {
                a_n += 1;
                burn_charged((null == 0).then(|| f32::from_bits(w)))
            } else {
                b_n += 1;
                burn_state_on((null == 0).then(|| w as u8))
            };
            assert_eq!(got, b == "1", "{l}");
        }
        assert_eq!((a_n, b_n), (2020, 258));
    }

    #[test]
    fn police_close_matches_the_original() {
        // tools/police_close_reference.py: F-91297 on 3,000 cases (empty slots, modes, underground flags, positions at the
        // radius, r 250 / random / 0, no player)
        let mut n = 0;
        for l in include_str!("testdata/police_close_ref.txt").lines() {
            let (a, b) = l.split_once(" | ").unwrap();
            let v: Vec<u32> = a.split_whitespace().map(|x| x.parse().unwrap()).collect();
            let f = |i: usize| f32::from_bits(v[i]);
            let player = (v[0] != 0).then(|| Vec3::new(f(2), f(3), f(4)));
            let units: Vec<Vec3> = (0..v[5] as usize).map(|k| 6 + 6 * k).filter(|&o| v[o] == 0 && v[o + 1] == 0 && v[o + 2] == 0).map(|o| Vec3::new(f(o + 3), f(o + 4), f(o + 5))).collect();
            assert_eq!(any_police_close(player, units, f(1)), b == "1", "{l}");
            n += 1;
        }
        assert_eq!(n, 3000);
    }

    #[test]
    fn path_change_matches_the_original() {
        // tools/path_change_reference.py: F-84839 on 4,000 cases (groups of 1 - 4 paths of 0 - 6 points, own index, timers,
        // percents, MWC states): result, the outs (2004318071 = untouched), the timer and the MWC after
        let mut n = 0;
        for l in include_str!("testdata/path_change_ref.txt").lines() {
            let (a, b) = l.split_once(" | ").unwrap();
            let v: Vec<&str> = a.split_whitespace().collect();
            let np: usize = v[0].parse().unwrap();
            let cnt: Vec<usize> = v[1].split(',').map(|x| x.parse().unwrap()).collect();
            let u = |i: usize| v[i].parse::<u32>().unwrap();
            let own = u(2) as i32;
            let mut timer = f32::from_bits(u(3));
            let pct = u(4) as i32;
            let mut rng = (u(5), u(6));
            let pos = Vec3::new(f32::from_bits(u(7)), f32::from_bits(u(8)), f32::from_bits(u(9)));
            let w: Vec<u32> = if v[10] == "-" { vec![] } else { v[10].split(',').map(|x| x.parse().unwrap()).collect() };
            let mut group = Vec::new();
            let mut q = 0;
            for j in 0..np {
                group.push((0..cnt[j]).map(|k| Vec3::new(f32::from_bits(w[3 * (q + k)]), f32::from_bits(w[3 * (q + k) + 1]), f32::from_bits(w[3 * (q + k) + 2]))).collect::<Vec<_>>());
                q += cnt[j];
            }
            let r: Vec<i64> = b.split_whitespace().map(|x| x.parse().unwrap()).collect();
            let got = path_change(&group, own, &mut timer, pos, pct, &mut rng);
            let (res, o1, o2) = match got { Some((j, k)) => (1, j as i64, k as i64), None => (0, 2004318071, 2004318071) };
            assert_eq!((res, o1, o2, timer.to_bits() as i64, rng.0 as i64, rng.1 as i64), (r[0], r[1], r[2], r[3], r[4], r[5]), "{l}");
            n += 1;
        }
        assert_eq!(n, 4000);
    }

    #[test]
    fn idle_ask_matches_the_original() {
        // tools/idle_ask_reference.py: F-44966 for commands 0 - 12 and four large / negative words: the state asked
        // (-2 none) and the machine's +16 after (-1 cleared, 7 untouched)
        let mut n = 0;
        for l in include_str!("testdata/idle_ask_ref.txt").lines() {
            let v: Vec<i64> = l.split_whitespace().map(|x| x.parse().unwrap()).collect();
            let (ask, clear) = idle_ask(v[0] as u32);
            assert_eq!(ask.map_or(-2, |s| s as i64), v[1], "{l}");
            assert_eq!(if clear { -1 } else { 7 }, v[2], "{l}");
            n += 1;
        }
        assert_eq!(n, 17);
    }

    #[test]
    fn closing_matches_the_original() {
        // tools/closing_reference.py: F-32902 on 6,000 cases (ranges 0 / 7 / 50 / at the distance, speeds at the floor, the
        // 0.9 edge, A on B, tiny offsets)
        let mut n = 0;
        for l in include_str!("testdata/closing_ref.txt").lines() {
            let w: Vec<u32> = l.split_whitespace().map(|x| x.parse().unwrap()).collect();
            let f = |i: usize| f32::from_bits(w[i]);
            let v3 = |i: usize| Vec3::new(f(i), f(i + 1), f(i + 2));
            assert_eq!(closing_in(v3(1), v3(4), f(7), f(8), v3(9), f(0), f(12)), w[13] == 1, "{l}");
            n += 1;
        }
        assert_eq!(n, 6000);
    }

    #[test]
    fn detect_pick_matches_the_original() {
        // tools/detect_pick_reference.py: F-76033 over 3,000 managers (1 - 5 units, empty slots, +1072 / +872 bytes, tactic and
        // sight answers, every need_check / need_free / group): the unit it returns
        let mut n = 0;
        for l in include_str!("testdata/detect_pick_ref.txt").lines() {
            let v: Vec<i64> = l.split_whitespace().map(|x| x.parse().unwrap()).collect();
            let (k, em, ck, gp, tm, sm) = (v[0] as usize, v[1], v[2], v[3], v[4], v[5]);
            let unit = |i: usize| ((em >> i) & 1 == 0).then(|| ((ck >> (2 * i)) & 3 != 0, (tm >> i) & 1 != 0, ((gp >> (2 * i)) & 3) as u8));
            let r = first_detecting(k, v[6] != 0, v[7] != 0, v[8] as u8, unit, |i| (sm >> i) & 1 != 0);
            assert_eq!(r.map_or(-1, |i| i as i64), v[9], "{l}");
            n += 1;
        }
        assert_eq!(n, 3000);
    }

    #[test]
    fn side_pick_matches_the_original() {
        // tools/side_pick_reference.py: F-14848 over all 32 answers of its five tests
        let mut n = 0;
        for l in include_str!("testdata/side_pick_ref.txt").lines() {
            let v: Vec<bool> = l.split_whitespace().map(|x| x == "1").collect();
            assert_eq!(side_pick(v[0], v[1], v[2], v[3], v[4]), v[5], "{l}");
            n += 1;
        }
        assert_eq!(n, 32);
    }

    #[test]
    fn pursuit_delay_matches_the_original() {
        // tools/pursuit_delay_reference.py: the original's Pursuit with a Delay hold, a still world; per hold the first update
        // that asks CatchUp, the first in CatchUp, and the Delay's time after its last update
        let mut n = 0;
        for l in include_str!("testdata/pursuit_delay_ref.txt").lines() {
            let v: Vec<i64> = l.split_whitespace().map(|x| x.parse().unwrap()).collect();
            let (mut hold, mut t, mut ask) = (f32::from_bits(v[0] as u32), 0.0f32, false);
            let (mut ka, mut kc, mut tb) = (if hold <= 0.0 { 0 } else { -1 }, if hold <= 0.0 { 0 } else { -1 }, 0u32);
            let mut k = 0;
            while kc < 0 {
                delay_step(&mut hold, &mut t, &mut ask, 1.0 / 60.0);
                if hold > 0.0 {
                    tb = t.to_bits();
                }
                if ka < 0 && (ask || hold <= 0.0) {
                    ka = k;
                }
                if hold <= 0.0 {
                    kc = k;
                }
                k += 1;
            }
            assert_eq!((ka, kc, tb as i64), (v[2], v[3], v[4]), "{l}");
            n += 1;
        }
        assert_eq!(n, 288);
    }

    /// StartTactic against the original's F-33550 run by tools/start_tactic_reference.py over every asked kind, tactic already on
    /// the target (none / its type, its byte +28), law record (none / score 0, 4002, 9999), show flag, racer and tutorial byte
    /// (1,760 cases): kept / refused / started, the old one ended, the duration carried, and the floor - only on a start, kind 1
    /// the table's entry 1 (iPullOverLawBreakScore), kinds 2 and 3 entry 3 (iPursuitLawBreakScore), kind 4 entry 0
    /// (iMaxLawBreakingScore), the harness's table 4001 .. 4004.
    /// The police strip's texts StartTactic asks for (F-76838, logged by tools/start_tactic_reference.py as 4x): on every
    /// start with the show flag (every caller passes 1), whatever the target - Dispatch 2, Pull-over 0 then 3, the others 1
    /// then 3; none on a refused or kept start.
    #[test]
    fn start_tactic_hud_texts_match_the_original() {
        let kinds = [TacKind::Dispatch, TacKind::PullOver, TacKind::Pursuit, TacKind::PursuitDbl, TacKind::ArrowTrap];
        let mut n = 0;
        for l in include_str!("testdata/start_tactic_ref.txt").lines().filter(|l| !l.starts_with("BLOCK")) {
            let parts: Vec<&str> = l.split('|').map(str::trim).collect();
            let v: Vec<i32> = parts[0].split_whitespace().map(|x| x.parse().unwrap()).collect();
            if v[4] != 1 {
                continue;
            }
            let log: Vec<&str> = parts[2].split_whitespace().skip(1).collect();
            let want: Vec<u8> = log.iter().filter(|c| c.len() == 2 && c.starts_with('4')).map(|c| c[1..].parse().unwrap()).collect();
            let mut p = Police::new();
            if v[1] >= 0 {
                p.start_tactic(kinds[v[1] as usize], vec![], v[2] != 0, 25.0);
            }
            p.hud_msgs.clear();
            p.start_tactic(kinds[v[0] as usize], vec![], true, 25.0);
            assert_eq!(p.hud_msgs, want, "{l}");
            n += 1;
        }
        assert_eq!(n, 880);
    }

    #[test]
    fn start_tactic_matches_the_original() {
        let reference = include_str!("testdata/start_tactic_ref.txt");
        let (mut n, mut carries, mut refusals) = (0, 0, 0);
        // (the harness's loops: kind, old, b28, record (none / 0 / 4002 / 9999), show, racer, tutorial; no b28 without an old)
        let mut ins = Vec::new();
        for _kind in 0..5 {
            for old in -1..5 {
                for b28 in 0..2 {
                    if old < 0 && b28 == 1 {
                        continue;
                    }
                    for rs in [None, Some(0), Some(4002), Some(9999)] {
                        for _ in 0..8 {
                            ins.push(rs);
                        }
                    }
                }
            }
        }
        let mut inputs = ins.into_iter();
        // (the port's floors are the tune's entries: PullOver iPullOverLawBreakScore, the pursuits iPursuitLawBreakScore,
        // the Arrow Trap iMaxLawBreakingScore - as `TacKind::floor`'s defaults)
        assert_eq!([TacKind::PullOver, TacKind::Pursuit, TacKind::PursuitDbl, TacKind::ArrowTrap].map(|k| k.floor()), [250.0, 600.0, 600.0, 1000.0]);
        // (the request blocks the Start* callers build, F-86662 / F-37640 on 64 bytes of A5: no target, record or cop, the four
        // flag bytes +12..+15 1 - the tactic's +28 may escalate, +29 track, +30 retarget, +31 - +16 the teleport lock 0.0, the
        // Pursuit's +20 delay 0.0. The port: may_escalate true from the law ladder, the manager's chase properties all on,
        // start_tactic's tel_lock 0 and no Delay unless a bolt from the ticket sets it)
        let blk = |name: &str| -> Vec<u32> {
            let l = reference.lines().find(|l| l.starts_with(&format!("BLOCK {name} "))).unwrap();
            l.split_whitespace().skip(2).map(|x| u32::from_str_radix(x, 16).unwrap()).collect()
        };
        let p = Police::new();
        for name in ["request", "pursuit"] {
            let b = blk(name);
            assert_eq!(&b[..3], &[0, 0, 0], "{name}");
            assert_eq!(b[3], 0x0101_0101, "{name}");
            assert_eq!(f32::from_bits(b[4]), p.tel_lock, "{name}");
            assert_eq!(p.chase_props, [b[3] >> 16 & 1 != 0, b[3] >> 8 & 1 != 0, b[3] & 1 != 0]);
        }
        assert_eq!(f32::from_bits(blk("pursuit")[5]), p.flee_hold);
        let (mut tuts, mut cpas) = (0, 0);
        for l in reference.lines().filter(|l| !l.starts_with("BLOCK")) {
            let parts: Vec<&str> = l.split('|').map(str::trim).collect();
            let v: Vec<i32> = parts[0].split_whitespace().map(|x| x.parse().unwrap()).collect();
            // (the side calls the port carries: the phone interrupt (code 1) for every kind but 0, started or not; the
            // tutorial (5) - every caller passes show 1, the target the player (racer 1), its +76 bit clear - as
            // `pursuit_tut` on a start)
            let log: Vec<&str> = parts[2].split_whitespace().skip(1).collect();
            assert_eq!(log.contains(&"1"), v[0] != 0, "{l}");
            if v[4] == 1 && v[5] == 1 && v[6] == 0 && log.iter().any(|c| c.starts_with('6')) {
                let kinds = [TacKind::Dispatch, TacKind::PullOver, TacKind::Pursuit, TacKind::PursuitDbl, TacKind::ArrowTrap];
                let mut p = Police::new();
                if v[1] >= 0 {
                    p.start_tactic(kinds[v[1] as usize], vec![], v[2] != 0, 25.0);
                }
                p.pursuit_tut = false;
                p.start_tactic(kinds[v[0] as usize], vec![], true, 25.0);
                assert_eq!(p.pursuit_tut, log.contains(&"5"), "{l}");
                tuts += p.pursuit_tut as u32;
            }
            // (the stat gCPA (code 2) and the event POLICE_PURSUIT (code 3) on every start: the port counts the stat; the
            // event is posted for exactly the kinds >= 2)
            if log.iter().any(|c| c.starts_with('6')) {
                let kinds = [TacKind::Dispatch, TacKind::PullOver, TacKind::Pursuit, TacKind::PursuitDbl, TacKind::ArrowTrap];
                let mut p = Police::new();
                if v[1] >= 0 {
                    p.start_tactic(kinds[v[1] as usize], vec![], v[2] != 0, 25.0);
                }
                p.stat_cpa = 0;
                p.start_tactic(kinds[v[0] as usize], vec![], true, 25.0);
                assert_eq!(p.stat_cpa == 1, log.contains(&"2"), "{l}");
                assert_eq!(v[0] >= 2, log.contains(&"3"), "{l}");
                cpas += p.stat_cpa;
            }
            let (kind, old, b28, score) = (v[0], v[1], v[2] != 0, v[3]);
            let ret: i32 = parts[1].strip_prefix("ret ").unwrap().parse().unwrap();
            let log: Vec<u32> = parts[2].split_whitespace().skip(1).map(|x| u32::from_str_radix(x, 16).unwrap()).collect();
            let carry_bits = u32::from_str_radix(parts[3].strip_prefix("+104 ").unwrap(), 16).unwrap();
            let started = log.iter().any(|&c| c == 0x60 + kind as u32);
            let ended = old >= 0 && log.contains(&(0x70 + old as u32));
            let orig = if !started && ret == -1 {
                StartRule::Refuse
            } else if !started {
                assert_eq!(ret, old, "{l}");
                StartRule::Keep
            } else {
                StartRule::Start { end_old: ended, carry: carry_bits == 7.25f32.to_bits() }
            };
            let port = start_rule(kind, (old >= 0).then_some((old, b28)));
            assert_eq!(port, orig, "{l}");
            // (the floor on a start: the score after = max(input, the kind's entry); the input from the harness's loop order)
            let input = inputs.next().unwrap();
            if let Some(sc) = input {
                let after = if matches!(port, StartRule::Start { .. }) && kind >= 1 { sc.max([0, 4002, 4004, 4004, 4001][kind as usize]) } else { sc };
                assert_eq!(score, after, "{l}");
            }
            n += 1;
            carries += matches!(port, StartRule::Start { carry: true, .. }) as u32;
            refusals += (port == StartRule::Refuse) as u32;
        }
        assert!(n == 1760 && carries > 0 && refusals > 0 && tuts > 0 && cpas == 256, "{n} {carries} {refusals} {tuts} {cpas}");
    }

    /// The Double Pursuit's may-escalate against the original's F-73000 (with F-38605 and the state tests F-48174 / F-12623 on
    /// the tactic's own vtable) run by tools/pursuit_dbl_escalate_reference.py: for each machine state, byte +28 and radio byte,
    /// the results over the same 65,548 arrest-timer patterns give the same true count and FNV-1a hash.
    #[test]
    fn pursuit_dbl_escalate_matches_the_original() {
        let reference = include_str!("testdata/pursuit_dbl_escalate_ref.txt");
        const SP: [u32; 12] = [0, 0x8000_0000, 1, 0x8000_0001, 0x3F80_0000, 0xBF80_0000, 0x7F80_0000, 0xFF80_0000, 0x7FC0_0000, 0xFFC0_0000, 0x7F80_0001, 0x007F_FFFF];
        let pats: Vec<u32> = SP.iter().copied().chain((0..65536u32).map(|k| k.wrapping_mul(65537))).collect();
        let mut lines = 0;
        for l in reference.lines() {
            let v: Vec<&str> = l.split_whitespace().collect();
            let (state, b28, radio): (i32, u32, u32) = (v[0].parse().unwrap(), v[1].parse().unwrap(), v[2].parse().unwrap());
            let (mut n, mut h) = (0u32, 2_166_136_261u32);
            for &p in &pats {
                let t = tactic_may_escalate(radio & 0xFF != 0, b28 & 0xFF != 0, state == 2, state == 1, f32::from_bits(p)) as u32;
                n += t;
                h = (h ^ t).wrapping_mul(16_777_619);
            }
            assert_eq!(format!("{n} {h:08x}"), format!("{} {}", v[3], v[4]), "{l}");
            lines += 1;
        }
        assert_eq!(lines, 72);
    }

    /// The brain command against the original's F-83687 run offline by tools/brain_command_reference.py: the same 4,000 calls
    /// from the same xorshift (command, block, the machine's state, the brain's words before), every changed brain word, the
    /// route clear, the machine's reset and the state asked equal.
    #[test]
    fn brain_command_matches_the_original() {
        let reference = include_str!("testdata/brain_command_ref.txt");
        let mut x = 20261106u32;
        let mut nx = || {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            x
        };
        const F: [u32; 8] = [0, 0x3F80_0000, 0xBF80_0000, 0xC47A_0000, 0x3F00_0000, 0x4020_0000, 0xBE80_0000, 0x3F00_0001];
        const I: [u32; 4] = [0, 1, 0xFFFF_FFFF, 0x0100_0000];
        let mut word = |nx: &mut dyn FnMut() -> u32| {
            let r = nx();
            match r % 10 {
                0..=2 => F[((r >> 8) % 8) as usize],
                3 | 4 => I[((r >> 8) % 4) as usize],
                _ if r >> 24 == 0x82 => r ^ 0x0100_0000,
                _ => r,
            }
        };
        // (the brain's words +568..+776 and the fields F-83687 writes, by offset; bytes are the word's high bytes)
        const LO: usize = 568;
        let get = |m: &[u32], o: usize| m[(o - LO) / 4];
        let byte = |m: &[u32], o: usize| (get(m, o & !3) >> (24 - 8 * (o & 3))) as u8;
        let read = |m: &[u32], f292: u32| BrainCmd {
            cmd: get(m, 568), t572: f32::from_bits(get(m, 572)), n576: get(m, 576), t580: f32::from_bits(get(m, 580)),
            t584: f32::from_bits(get(m, 584)), t588: f32::from_bits(get(m, 588)), b592: byte(m, 592), b593: byte(m, 593),
            unseen: f32::from_bits(get(m, 596)), cone_t: f32::from_bits(get(m, 600)), b604: byte(m, 604), b605: byte(m, 605),
            tactic: get(m, 676), tgt: [get(m, 680), get(m, 684)], stop: [get(m, 688), get(m, 692), get(m, 696), get(m, 700)],
            scale: f32::from_bits(get(m, 704)), side: byte(m, 708), side_d: f32::from_bits(get(m, 712)), align: f32::from_bits(get(m, 716)),
            place: [get(m, 720), get(m, 724), get(m, 728), get(m, 732)], srch_side: byte(m, 736), srch_ang: f32::from_bits(get(m, 740)),
            b772: byte(m, 772), final_speed: f32::from_bits(f292),
        };
        let write = |m: &mut [u32], b: &BrainCmd| {
            let mut put = |o: usize, v: u32| m[(o - LO) / 4] = v;
            for (o, v) in [(568, b.cmd), (572, b.t572.to_bits()), (576, b.n576), (580, b.t580.to_bits()), (584, b.t584.to_bits()),
                           (588, b.t588.to_bits()), (596, b.unseen.to_bits()), (600, b.cone_t.to_bits()), (676, b.tactic), (680, b.tgt[0]),
                           (684, b.tgt[1]), (688, b.stop[0]), (692, b.stop[1]), (696, b.stop[2]), (700, b.stop[3]), (704, b.scale.to_bits()),
                           (712, b.side_d.to_bits()), (716, b.align.to_bits()), (720, b.place[0]), (724, b.place[1]), (728, b.place[2]),
                           (732, b.place[3]), (740, b.srch_ang.to_bits())] {
                put(o, v);
            }
            let mut putb = |o: usize, v: u8| {
                let s = 24 - 8 * (o & 3);
                let i = ((o & !3) - LO) / 4;
                m[i] = (m[i] & !(0xFF << s)) | ((v as u32) << s);
            };
            for (o, v) in [(592, b.b592), (593, b.b593), (604, b.b604), (605, b.b605), (708, b.side), (736, b.srch_side), (772, b.b772)] {
                putb(o, v);
            }
        };
        let mut lines = reference.lines();
        // (the blocks the senders build, against the original's initialisers F-59773 / F-72181 / F-94949 / F-60530 / F-11391 on 32 bytes of A5,
        // on the words F-83687 reads for that command: +0..+12, and Stop's byte +16 and +20..+28, AlongSide's byte +16 and +20;
        // the image's fAlongSideDist is the class default 3.5)
        for (name, blk, read) in [("basic", cmd_block(4, false, 0.0, false, 0.0), &[0, 1, 2, 3][..]),
                                  ("stop", cmd_block(1, false, 0.0, false, 0.0), &[0, 1, 2, 3, 4, 5, 6, 7][..]),
                                  ("along", cmd_block(5, false, 0.0, false, 3.5), &[0, 1, 2, 3, 4, 5][..]),
                                  ("search", search_block(false, 0.0), &[0, 1, 2, 3, 4, 5][..]),
                                  // (Respond's F-11391: the vector +16..+31 is the global's, zero in the image - the sender
                                  // writes the place over it; the port's block for a place at the origin)
                                  ("respond", respond_block(Vec3::ZERO), &[0, 1, 2, 3, 4, 5, 6, 7][..])] {
            let v: Vec<&str> = lines.next().unwrap().split_whitespace().collect();
            assert_eq!((v[0], v[1]), ("BLOCK", name));
            for &i in read {
                let want = u32::from_str_radix(v[2 + i], 16).unwrap();
                let (want, got) = if i == 4 && name != "respond" { (want >> 24, blk[i] >> 24) } else { (want, blk[i]) };
                assert_eq!(got, want, "{name} word {i}");
            }
        }
        let mut changed_any = 0;
        for k in 0..4000 {
            let cmd = nx() % 10;
            let none = cmd == 0 && nx() % 3 == 0;
            let cur = match nx() % 3 { 0 => 0, 1 => -1, _ => 3 };
            let blk: Option<[u32; 8]> = (!none).then(|| std::array::from_fn(|_| word(&mut nx)));
            let mut pre: Vec<u32> = (LO..780).step_by(4).map(|_| word(&mut nx)).collect();
            let prev = if nx() % 4 == 0 { 0 } else { nx() % 9 };
            pre[0] = prev;
            let f292 = word(&mut nx);
            let mut b = read(&pre, f292);
            let (route, reset, st) = brain_command(&mut b, cmd, blk.as_ref(), cur);
            let mut post = pre.clone();
            write(&mut post, &b);
            let mut got = format!("{k} {cmd}");
            for (i, (p, q)) in pre.iter().zip(&post).enumerate() {
                if p != q {
                    got += &format!(" {}={q:08x}", LO + 4 * i);
                    changed_any += 1;
                }
            }
            if b.final_speed.to_bits() != f292 {
                got += &format!(" 292={:08x}", b.final_speed.to_bits());
            }
            got += &format!(" route={} reset={} state={}", route as u8, if reset { 1 } else { -1 }, st as i32);
            assert_eq!(got, lines.next().unwrap(), "call {k}");
        }
        assert!(changed_any > 30000, "{changed_any}");
    }

    /// The squad swipes against the original's: the Arrow Trap's CatchUp update F-47983 and the Double Pursuit's AlongSide
    /// update F-41674 run offline by tools/squad_swipe_reference.py on random cop geometry (testdata/squad_swipe_ref.txt):
    /// every update's overrides (steer, brake, handbrake bits, or none), the wait +24's bits, the latch +32 and the line count
    /// (4 of the 16 cases with cop 0's +872 = 0, as the payback's goons: no line).
    #[test]
    fn squad_swipes_match_the_original() {
        let reference = include_str!("testdata/squad_swipe_ref.txt");
        let hex = |s: &str| f32::from_bits(u32::from_str_radix(s, 16).unwrap());
        let v3 = |v: &[&str]| Vec3::new(hex(v[0]), hex(v[1]), hex(v[2]));
        let (mut checked, mut swiped) = (0, 0);
        let (mut arrow, mut law, mut a, mut b, mut lines) = (false, true, 0, 0, 0u32);
        let (mut cops, mut head_on): (Vec<[Vec3; 4]>, Vec<bool>) = (Vec::new(), Vec::new());
        let (mut tpos, mut tvel) = (Vec3::ZERO, Vec3::ZERO);
        let mut sq = Squad::default();
        let mut want = String::new();
        for line in reference.lines() {
            let v: Vec<&str> = line.split_whitespace().collect();
            match v[0] {
                "CASE" => {
                    (arrow, law) = (v[1] == "1", v[5] == "1");
                    let n: usize = v[2].parse().unwrap();
                    (a, b) = (v[3].parse().unwrap(), v[4].parse().unwrap());
                    (cops, head_on, sq, lines) = (vec![[Vec3::ZERO; 4]; n], vec![false; n], Squad::default(), 0);
                }
                "G" => {
                    let k: usize = v[1].parse().unwrap();
                    cops[k] = [v3(&v[2..]), v3(&v[5..]), v3(&v[8..]), v3(&v[11..])];
                    head_on[k] = v[14] == "1";
                }
                "T" => (tpos, tvel) = (v3(&v[1..]), v3(&v[4..])),
                "END" => {}
                _ => {
                    if v[0] != "=" {
                        want = line.to_string();
                    }
                    let g: Vec<Option<SwipeGeo>> = cops.iter().map(|c| Some(swipe_geo(c[0], c[1], c[2], c[3], tpos, tvel))).collect();
                    let mut ovr = vec![crate::ai::Overrides::default(); cops.len()];
                    // (the line only for a law tactic: F-37230, cop 0's +872 - the caller's `!goons`)
                    lines += (squad_swipes(&mut sq, arrow, 1.0 / 60.0, Some(a), Some(b), &g, &head_on, 60.0, &mut ovr) && law) as u32;
                    let bits = |o: Option<f32>| o.map_or(0xDEADBEEF, f32::to_bits);
                    let mut got = String::new();
                    for o in &ovr {
                        got += &format!("{:08x} {:08x} {:08x} ", bits(o.steer), bits(o.brake), bits(o.handbrake.map(|_| 1.0)));
                        swiped += o.brake.is_some() as u32;
                    }
                    got += &format!("{:08x} {} {}", sq.swipe_t.to_bits(), sq.latch as u8, lines);
                    assert_eq!(got, want, "update {checked}");
                    checked += 1;
                }
            }
        }
        assert!(checked == 4800 && swiped > 400, "{checked} {swiped}");
    }

    /// The swipes against a recording of the running original: a Double Pursuit (StartPursuit kind 2) on the player's car
    /// driven along Hollywood Blvd, recorded at the entry and the exit of its update F-40655 (docs/reference/hookmen/
    /// double_pursuit_recording.md). Every AlongSide update: the recorded dt, the state's wait +24 and latch +32, the flank
    /// slots, each cop's place, rows 0 / 2, velocity and head-on byte and the target's place and velocity in; the port's
    /// swipes over the brains' override words at the entry (+756 steer, +748 brake, +752 handbrake - the brain resets them
    /// each frame) must give the words, the wait and the latch the update left.
    #[test]
    fn squad_swipes_match_the_recording() {
        let rec = include_str!("testdata/squad_swipe_rec.txt");
        let hex = |s: &str| u32::from_str_radix(s, 16).unwrap();
        let fl = |s: &str| f32::from_bits(hex(s));
        let v3 = |v: &[&str]| Vec3::new(fl(v[0]), fl(v[1]), fl(v[2]));
        let mut lines = rec.lines();
        let (mut checked, mut swiped) = (0, 0);
        while let Some(u) = lines.next() {
            let u: Vec<&str> = u.split_whitespace().collect();
            assert_eq!(u[0], "U");
            let dt = fl(u[1]);
            let mut sq = Squad { swipe_t: fl(u[2]), latch: hex(u[3]) != 0, ..Default::default() };
            let slot = |s: &str| (hex(s) as i32 >= 0).then(|| hex(s) as usize);
            let (a, b, n) = (slot(u[4]), slot(u[5]), u[6].parse::<usize>().unwrap());
            let cops: Vec<Vec<&str>> = (0..n).map(|_| lines.next().unwrap().split_whitespace().collect()).collect();
            let t: Vec<&str> = lines.next().unwrap().split_whitespace().collect();
            let w: Vec<&str> = lines.next().unwrap().split_whitespace().collect();
            let (tpos, tvel) = (v3(&t[1..]), v3(&t[4..]));
            let g: Vec<Option<SwipeGeo>> = cops.iter().map(|c| Some(swipe_geo(v3(&c[1..]), v3(&c[4..]), v3(&c[7..]), v3(&c[10..]), tpos, tvel))).collect();
            let head_on: Vec<bool> = cops.iter().map(|c| c[13] != "0").collect();
            let mut ovr = vec![crate::ai::Overrides::default(); n];
            squad_swipes(&mut sq, false, dt, a, b, &g, &head_on, 60.0, &mut ovr);
            let mut got = format!("W {:08X} {:02X}", sq.swipe_t.to_bits(), sq.latch as u8);
            for (c, o) in cops.iter().zip(&ovr) {
                let word = |v: Option<f32>, i: usize| v.map_or(hex(c[i]), f32::to_bits);
                got += &format!(" {:08X} {:08X} {:08X}", word(o.steer, 14), word(o.brake, 15), word(o.handbrake.map(|_| 1.0), 16));
                swiped += o.brake.is_some() as u32;
            }
            assert_eq!(got, w.join(" "), "update {checked}");
            checked += 1;
        }
        assert!(checked == 2355 && swiped == 190, "{checked} {swiped}");
    }

    /// The Pursuit's machine against a recording of the running original (docs/reference/hookmen/pursuit_recording.md): a
    /// Pursuit (StartPursuit kind 1) on the player's car, recorded at the entry and the exit of F-68032. Each update in
    /// CatchUp / AlongSide / Block / Halt starts the port from the recorded state (state, asked state, its time, AlongSide's
    /// +16 / +20 or Halt's +16, the brain's command / handbrake / side bytes, the tactic's +64), computes the tests from the
    /// recorded cop and target frames, velocities and chassis speeds (`swipe_geo`, `pursuit_sees`, `closing_in`) and runs
    /// `pursuit_machine` once: state, asked state, time bits, timers, command, handbrake and side must be the exit's. Updates
    /// whose entry or exit is Delay, Search or Arrest are the heat's (counted, not run). The side pick F-14848 (mesh / road
    /// tests) is taken from the recording: `pick` is the exit's side byte.
    #[test]
    fn pursuit_machine_matches_the_recording() {
        let rec = include_str!("testdata/pursuit_rec.txt");
        let hex = |s: &str| u32::from_str_radix(s, 16).unwrap();
        let fl = |s: &str| f32::from_bits(hex(s));
        let v3 = |v: &[&str]| Vec3::new(fl(v[0]), fl(v[1]), fl(v[2]));
        let tac_of = |id: i32| match id {
            1 => Some(Tac::CatchUp),
            2 => Some(Tac::AlongSide),
            3 => Some(Tac::Block),
            5 => Some(Tac::Halt),
            _ => None,
        };
        let code = |t: Tac| match t {
            Tac::CatchUp => 1,
            Tac::AlongSide => 2,
            Tac::Block => 3,
            Tac::Halt => 5,
            Tac::Avoid => 9,
        };
        let (safe, along, moving_speed, thresh) = (30.0f32, 5.0f32, 1.0f32, 10.0f32);
        let (mut checked, mut heat, mut states) = (0, 0, [0u32; 6]);
        for line in rec.lines() {
            let v: Vec<&str> = line.split_whitespace().collect();
            assert_eq!((v[0], v[37]), ("P", "="));
            let int = |i: usize| v[i].parse::<i32>().unwrap();
            let (st, asked, ost, oasked) = (int(4), int(5), int(38), int(39));
            let (Some(s0), Some(_)) = (tac_of(st), tac_of(ost)) else {
                heat += 1;
                continue;
            };
            if asked >= 0 && tac_of(asked).is_none() || oasked >= 0 && tac_of(oasked).is_none() {
                heat += 1;
                continue;
            }
            let mut tac = CopTac {
                st: s0,
                t: fl(v[6]),
                begun: true,
                next: tac_of(asked),
                cmd: int(9) as u8,
                hb: int(10) != 0,
                side: int(11) != 0,
                ..Default::default()
            };
            match s0 {
                Tac::AlongSide => (tac.far_t, tac.side_t) = (fl(v[7]), fl(v[8])),
                Tac::Halt => tac.halt_steer = fl(v[7]),
                _ => {}
            }
            let (cpos, crow0, crow2, cvel, cspeed) = (v3(&v[14..]), v3(&v[17..]), v3(&v[20..]), v3(&v[23..]), fl(v[26]));
            let (tpos, trow2, tvel, tspeed) = (v3(&v[27..]), v3(&v[30..]), v3(&v[33..]), fl(v[36]));
            let geo = swipe_geo(cpos, crow0, crow2, cvel, tpos, tvel);
            let closing = closing_in(tpos, cpos, tspeed, cspeed, cvel, safe, moving_speed);
            let sees = pursuit_sees(&geo, &[geo.dsq], safe, along, crow2, trow2, tspeed, moving_speed, closing, int(45) != 0);
            pursuit_machine(&mut tac, &sees, thresh, fl(v[2]), fl(v[1]));
            let (a, z) = match tac.st {
                Tac::AlongSide => (tac.far_t.to_bits(), tac.side_t.to_bits()),
                Tac::Halt => (tac.halt_steer.to_bits(), 0),
                _ => (0, 0),
            };
            let got = format!("{} {} {:08X} {:08X} {:08X} {} {} {}", code(tac.st), tac.next.map_or(-1, code), tac.t.to_bits(), a, z, tac.cmd,
                              tac.hb as u8, tac.side as u8);
            assert_eq!(got, v[38..46].join(" "), "update {checked}: {line}");
            states[code(tac.st) as usize] += 1;
            checked += 1;
        }
        assert!(checked == 3572 && heat == 2347 && states == [0, 2099, 902, 70, 0, 501], "{checked} {heat} {states:?}");
    }

    /// The pull-over's machine against a recording of the running original (docs/reference/hookmen/pullover_recording.md): a
    /// pull-over (the manager's request / start F-36876 tactic on the player's car), recorded at the entry and the exit of
    /// F-36876. Each update in CatchUp / AvoidCollision / Halt starts the port from the recorded state (state, asked state,
    /// time, AvoidCollision's side timer or Halt's last squared distance and growing time, the brain's command / handbrake /
    /// side bytes), computes the tests from the recorded cop and target frames, velocities and chassis speeds (`swipe_geo`,
    /// `closing_in`, `moving`) and runs `pullover_machine` once: state, asked state, time, timers, command, handbrake and side
    /// must be the exit's. Updates whose entry or exit is Ticket are the heat's (counted, not run). The side pick F-14848 is
    /// taken from the recording (the exit's side byte); the re-pick gate is passed as true (open).
    #[test]
    fn pullover_machine_matches_the_recording() {
        let rec = include_str!("testdata/pullover_rec.txt");
        let hex = |s: &str| u32::from_str_radix(s, 16).unwrap();
        let fl = |s: &str| f32::from_bits(hex(s));
        let v3 = |v: &[&str]| Vec3::new(fl(v[0]), fl(v[1]), fl(v[2]));
        let tac_of = |id: i32| match id {
            0 => Some(Tac::CatchUp),
            1 => Some(Tac::Avoid),
            2 => Some(Tac::Halt),
            _ => None,
        };
        let code = |t: Tac| match t {
            Tac::CatchUp => 0,
            Tac::Avoid => 1,
            Tac::Halt => 2,
            _ => 9,
        };
        let (pull, moving_speed) = (15.0f32, 1.0f32);
        let (mut checked, mut heat, mut states) = (0, 0, [0u32; 3]);
        for line in rec.lines() {
            let v: Vec<&str> = line.split_whitespace().collect();
            assert_eq!((v[0], v[37]), ("V", "="));
            let int = |i: usize| v[i].parse::<i32>().unwrap();
            let (st, asked, ost, oasked) = (int(4), int(5), int(38), int(39));
            let (Some(s0), Some(_)) = (tac_of(st), tac_of(ost)) else {
                heat += 1;
                continue;
            };
            if asked >= 0 && tac_of(asked).is_none() || oasked >= 0 && tac_of(oasked).is_none() {
                heat += 1;
                continue;
            }
            let mut tac = CopTac {
                st: s0,
                t: fl(v[6]),
                begun: true,
                next: tac_of(asked),
                cmd: int(9) as u8,
                hb: int(10) != 0,
                side: int(11) != 0,
                ..Default::default()
            };
            match s0 {
                Tac::Avoid => tac.side_t = fl(v[7]),
                Tac::Halt => (tac.halt_d2, tac.grow_t) = (fl(v[7]), fl(v[8])),
                _ => {}
            }
            let (cpos, crow0, crow2, cvel, cspeed) = (v3(&v[14..]), v3(&v[17..]), v3(&v[20..]), v3(&v[23..]), fl(v[26]));
            let (tpos, tvel, tspeed) = (v3(&v[27..]), v3(&v[33..]), fl(v[36]));
            let geo = swipe_geo(cpos, crow0, crow2, cvel, tpos, tvel);
            let closing = |r: f32| closing_in(tpos, cpos, tspeed, cspeed, cvel, r, moving_speed);
            let (dx, dy, dz) = (tpos.x - cpos.x, tpos.y - cpos.y, tpos.z - cpos.z);
            let sees = PullSees {
                all_halt: geo.dsq < (pull - 1.0) * (pull - 1.0),
                moving: moving(tspeed, moving_speed),
                closing0: closing(0.0),
                closing7: closing(7.0),
                closing50: closing(50.0),
                near50: geo.dsq < 50.0 * 50.0,
                near_pull: geo.dsq < pull * pull,
                pick: int(45) != 0,
                gate: true,
                cspeed,
                d2: fmas(dx, dx, fmas(dz, dz, dy * dy)),
            };
            pullover_machine(&mut tac, &sees, fl(v[1]));
            let (a, z) = match tac.st {
                Tac::Avoid => (tac.side_t.to_bits(), 0),
                Tac::Halt => (tac.halt_d2.to_bits(), tac.grow_t.to_bits()),
                _ => (0, 0),
            };
            let got = format!("{} {} {:08X} {:08X} {:08X} {} {} {}", code(tac.st), tac.next.map_or(-1, code), tac.t.to_bits(), a, z, tac.cmd,
                              tac.hb as u8, tac.side as u8);
            assert_eq!(got, v[38..46].join(" "), "update {checked}: {line}");
            states[code(tac.st) as usize] += 1;
            checked += 1;
        }
        assert!(checked == 1908 && heat == 0 && states == [913, 195, 800], "{checked} {heat} {states:?}");
    }

    /// The cop brains' goal plans against recordings of the running original (docs/reference/hookmen/goal_plan_recording.md):
    /// every replan of F-67303 in a recorded Pursuit (Pursuit and AlongSide brains) and pull-over (PullOver brain), at its
    /// entry and exit. Each runs `cop_plan` on the city's nav data with the recorded command, head-on and side bytes, the
    /// last plan's heading (+656) and speed (+672), the countdown (+588), the cop's and the target's frames and velocities
    /// and the target's chassis speed: the two goals' bits and the final speed (+208) must be the exit's. Skipped without
    /// the data.
    #[test]
    fn cop_plan_matches_the_recording() {
        let dir = std::env::var("MCLA_DATA_DIR").unwrap_or_else(|_| "D:/MCLA/extracted/cache".to_string());
        let Ok(res) = crate::assets::load_resource_file(&std::path::Path::new(&dir).join("resources/city/sc/0x9960fcd8.r66")) else { return };
        let Ok(nav) = NavSys::parse(&res) else { return };
        if crate::offroad::mesh().is_none() {
            return;
        }
        let rec = include_str!("testdata/cop_plan_rec.txt");
        let hex = |s: &str| u32::from_str_radix(s, 16).unwrap();
        let fl = |s: &str| f32::from_bits(hex(s));
        let v3 = |v: &[&str]| Vec3::new(fl(v[0]), fl(v[1]), fl(v[2]));
        let (per, safe, along) = (1.0f32, 30.0f32, 5.0f32);
        let (mut n, mut bad) = (0, Vec::new());
        for line in rec.lines() {
            let v: Vec<&str> = line.split_whitespace().collect();
            assert_eq!((v[0], v[36]), ("G", "="));
            let int = |i: usize| v[i].parse::<u32>().unwrap();
            let (cmd, ho, side) = (int(1) as u8, int(2) != 0, int(4) as u8);
            let (t588, h656, s672) = (fl(v[5]), v3(&v[6..]), fl(v[9]));
            let (cpos, crow0, crow2, cvel) = (v3(&v[11..]), v3(&v[14..]), v3(&v[17..]), v3(&v[20..]));
            let (tpos, trow0, trow2, tvel, tspeed) = (v3(&v[23..]), v3(&v[26..]), v3(&v[29..]), v3(&v[32..]), fl(v[35]));
            // (the law record's frame bits and the target's +624 / +640: recorded from v6 on; the earlier lines are
            // Pursuit / AlongSide plans, which do not read them)
            let known = v[49] != "-";
            let wrong_way = known && hex(v[49]) & 0x2000 != 0;
            let lane_tan = (known && hex(v[53]) != 0).then(|| v3(&v[50..]));
            let pfwd = -trow2;
            let turned = h656 != Vec3::ZERO && (-h656).angle_between(pfwd) > 0.5236;
            let changed = (tspeed - s672).abs() > 6.705;
            let was_ho = int(3) != 0;
            // (the recording has no route state: a replan that is neither of these was the route's last goal reached)
            let normal = t588 == 0.0 || (was_ho && !ho);
            let quick = !normal && t588 < 0.3 * per && (turned || changed);
            let geo = swipe_geo(cpos, crow0, crow2, cvel, tpos, tvel);
            let pin = PlanIn {
                bcmd: cmd,
                ed78: true,
                head_on: ho,
                quick,
                turned,
                t_moving: moving(tspeed, 1.0),
                same_dir: geo.same_dir,
                ahead: geo.ahead,
                wrong_way,
                lane_tan,
                side,
                ppos: tpos,
                pvel: tvel,
                pfwd,
                pright: trow0,
                cpos,
                last_seen: v3(&v[46..]),
                speed: tspeed,
                d: (tpos - cpos).length(),
                safe,
                along,
                per,
                moving_speed: 1.0,
                pull_dist: 15.0,
            };
            let (g, fs) = cop_plan(Some(&nav), &pin);
            let got = format!("{:08X} {:08X} {:08X} {:08X} {:08X} {:08X} {:08X}", fs.to_bits(), g[0].x.to_bits(), g[0].y.to_bits(), g[0].z.to_bits(),
                              g[1].x.to_bits(), g[1].y.to_bits(), g[1].z.to_bits());
            let want = format!("{} {}", v[38], v[40..46].join(" "));
            if got != want {
                bad.push(format!("plan {n} (cmd {cmd} quick {quick} turned {turned}): want {want}\n                                 got  {got}"));
            }
            n += 1;
        }
        assert!(bad.is_empty(), "{} of {n} differ:\n{}", bad.len(), bad.join("\n"));
    }

    /// The brains' bearing cache against recordings of the running original (docs/reference/hookmen/squad_fsm.md, the
    /// bearing cache): the cache (+760 value, +764 set) is cleared by the brain update F-74140 and filled by F-90304 on
    /// the first ask. In the recorded Double Pursuit (d4) and Arrow Trap (a2) it is clear at every cop's entry to the
    /// tactic update (20,447), so the tactic is the first to ask; each value it left (1,347) must be the port's fresh
    /// bearing (`swipe_geo`: aiv::ang3 of the cop's right row and target - cop, x 180 / pi) from the entry's frames.
    #[test]
    fn bearing_cache_matches_the_recording() {
        let rec = include_str!("testdata/bearing_cache_rec.txt");
        let fl = |s: &str| f32::from_bits(u32::from_str_radix(s, 16).unwrap());
        let v3 = |v: &[&str]| Vec3::new(fl(v[0]), fl(v[1]), fl(v[2]));
        let mut n = 0;
        for line in rec.lines() {
            let v: Vec<&str> = line.split_whitespace().collect();
            assert_eq!(v[0], "B");
            let (right, pos, tpos) = (v3(&v[1..]), v3(&v[4..]), v3(&v[7..]));
            let g = swipe_geo(pos, right, Vec3::Z, Vec3::ZERO, tpos, Vec3::ZERO);
            assert_eq!(format!("{:08X}", g.bearing.to_bits()), v[10], "line {n}: {line}");
            n += 1;
        }
        assert_eq!(n, 1347);
    }

    /// The search's point lists against a recording of the running original (docs/reference/hookmen/search_recording.md):
    /// every build of F-52110 (first builds and rebuilds) recorded at its entry and exit in Pursuit and Arrow Trap
    /// searches. Each runs `search_points_on` on the city's navigation data (resources/city/sc: the r66 nav system, the r26
    /// mesh) with the recorded start, the target's last known velocity (+624), +736 / +740 and the shared generator's two
    /// words (the multiply-with-carry 0x5CDCFAA7): the count, every point's bits and whether it has a node must be the
    /// exit's. The vertical probe F-20855 (the city's collision) answers as it did in the recording, in the order the
    /// builder asked it, and the port must ask it as many times. Skipped without the data.
    #[test]
    fn search_points_match_the_recording() {
        let dir = std::env::var("MCLA_DATA_DIR").unwrap_or_else(|_| "D:/MCLA/extracted/cache".to_string());
        let Ok(res) = crate::assets::load_resource_file(&std::path::Path::new(&dir).join("resources/city/sc/0x9960fcd8.r66")) else { return };
        let Ok(nav) = NavSys::parse(&res) else { return };
        let Some(mesh) = crate::offroad::mesh() else { return };
        let rec = include_str!("testdata/search_points_rec.txt");
        let hex = |s: &str| u32::from_str_radix(s, 16).unwrap();
        let fl = |s: &str| f32::from_bits(hex(s));
        let v3 = |v: &[&str]| Vec3::new(fl(v[0]), fl(v[1]), fl(v[2]));
        let (mut checked, mut probed, mut bad) = (0, 0, Vec::new());
        for line in rec.lines() {
            let v: Vec<&str> = line.split_whitespace().collect();
            assert_eq!((v[0], v[12]), ("Q", "="));
            let (first, random, angle) = (v[1] == "1", v[2] == "1", fl(v[3]));
            let (lv, start) = (v3(&v[4..]), v3(&v[7..]));
            let (mut x, mut c) = (hex(v[10]) as u64, hex(v[11]) as u64);
            let mut rnd = || {
                let t = x * 0x5CDC_FAA7 + c;
                (x, c) = (t & 0xFFFF_FFFF, t >> 32);
                (x as u32 & 0x7F_FFFF) as f32 * (1.0 / 8_388_608.0)
            };
            // (the recorded answers of the vertical probe, in the order the builder asked them)
            let n: usize = v[14].parse().unwrap();
            let pv = &v[15 + 4 * n..];
            assert_eq!(pv[0], "P");
            let answers: Vec<Option<Vec3>> = (0..pv[1].parse::<usize>().unwrap()).map(|i| (hex(pv[2 + 4 * i]) != 0).then(|| v3(&pv[3 + 4 * i..]))).collect();
            let asked = std::cell::Cell::new(0usize);
            let probe = |_: Vec3, _: f32| -> Option<Vec3> {
                let i = asked.get();
                asked.set(i + 1);
                answers.get(i).copied().flatten()
            };
            let pts = search_points_on(&nav, Some(mesh), &probe, start, lv, 1.0, first, random, angle, &mut rnd);
            let got = std::iter::once(format!("{} {}", (pts.is_empty() as u8), pts.len()))
                .chain(pts.iter().map(|(p, n)| format!("{:08X} {:08X} {:08X} {}", p.x.to_bits(), p.y.to_bits(), p.z.to_bits(), n.is_some() as u8)))
                .collect::<Vec<_>>()
                .join(" ");
            let want = format!("{} {}", u8::from(hex(v[13]) != 0), v[14..15 + 4 * n].join(" "));
            probed += (asked.get() > 0) as u32;
            if got != want || asked.get() != answers.len() {
                bad.push(format!("build {checked} (probes {} of {}): {line}\n   got {got}", asked.get(), answers.len()));
            }
            checked += 1;
        }
        assert!(bad.is_empty(), "{} of {checked} differ ({probed} asked the probe):\n{}", bad.len(), bad.join("\n"));
        eprintln!("search points: {checked} builds equal, {probed} asked the probe");
    }

    /// The Arrow Trap against a recording of the running original (docs/reference/hookmen/arrow_trap_recording.md): an Arrow
    /// Trap (StartPursuit kind 3) on the player's car, recorded at the entry and the exit of F-76774. Each update in CatchUp /
    /// Halt starts the port from the recorded machine (state, asked state, CatchUp's lock / hysteresis / wait / latch, the
    /// three slots) and brains (command +568, side +708, head-on +774, the override words), gives it the recorded cops'
    /// frames, velocities and route points and the target's frame, velocity and chassis speed, and runs `squad_machine` with
    /// `squad_formation` and `squad_swipes` once: state, asked state and slots must be the exit's; after a CatchUp update also
    /// the lock, hysteresis, wait and latch bits and every cop's command, side and override words.
    #[test]
    fn arrow_trap_matches_the_recording() {
        let rec = include_str!("testdata/arrow_trap_rec.txt");
        let hex = |s: &str| u32::from_str_radix(s, 16).unwrap();
        let fl = |s: &str| f32::from_bits(hex(s));
        let v3 = |v: &[&str]| Vec3::new(fl(v[0]), fl(v[1]), fl(v[2]));
        let slot = |s: &str| s.parse::<i32>().ok().filter(|&k| k >= 0).map(|k| k as u32);
        let (safe, along, moving_speed, thresh, head_on_dist) = (30.0f32, 5.0f32, 1.0f32, 10.0f32, 60.0f32);
        let mut lines = rec.lines();
        let (mut checked, mut catchup, mut sends, mut swiped) = (0, 0, 0, 0);
        while let Some(u) = lines.next() {
            let u: Vec<&str> = u.split_whitespace().collect();
            assert_eq!(u[0], "A");
            let dt = fl(u[1]);
            let (st, asked) = (u[2].parse::<i32>().unwrap(), u[3].parse::<i32>().unwrap());
            let mut sq = Squad {
                begun: st == 0,
                halt: st == 2,
                next: (asked >= 0).then_some(asked == 2),
                lock: fl(u[4]),
                hyst: fl(u[5]),
                swipe_t: fl(u[6]),
                latch: hex(u[7]) != 0,
                slots: [slot(u[8]), slot(u[9]), slot(u[10])],
            };
            let n: usize = u[11].parse().unwrap();
            let cops: Vec<Vec<&str>> = (0..n).map(|_| lines.next().unwrap().split_whitespace().collect()).collect();
            let t: Vec<&str> = lines.next().unwrap().split_whitespace().collect();
            let w: Vec<&str> = lines.next().unwrap().split_whitespace().collect();
            let (tpos, tright, tback, tvel, tspeed) = (v3(&t[1..]), v3(&t[4..]), v3(&t[7..]), v3(&t[10..]), fl(t[13]));
            let pos: Vec<Vec3> = cops.iter().map(|c| v3(&c[1..])).collect();
            let g: Vec<Option<SwipeGeo>> = cops.iter().map(|c| Some(swipe_geo(v3(&c[1..]), v3(&c[4..]), v3(&c[7..]), v3(&c[10..]), tpos, tvel))).collect();
            let head_on: Vec<bool> = cops.iter().map(|c| c[13] != "0").collect();
            let route: Vec<Option<Vec3>> = cops.iter().map(|c| (c[16] == "1").then(|| v3(&c[17..]))).collect();
            let mut brains: Vec<(u8, bool)> = cops.iter().map(|c| (c[14].parse().unwrap(), c[15] != "0")).collect();
            let any_safe = g.iter().flatten().any(|x| x.dsq < safe * safe);
            let t_moving = moving(tspeed, moving_speed);
            let fast = tspeed * 2.237 > thresh;
            let mut ovr = vec![crate::ai::Overrides::default(); n];
            let mut sent = false;
            squad_machine(&mut sq, &mut (), true, any_safe, t_moving, fast, dt, &mut |sq: &mut Squad, _: &mut (), d: f32, enter: bool| {
                let mut slots = sq.slots.map(|k| k.map(|k| k as usize));
                let s = squad_formation(&mut sq.hyst, &mut slots, true, &mut brains, &pos, &route, tpos, tright, tback, along, d, enter);
                sq.slots = slots.map(|k| k.map(|k| k as u32));
                sent |= s;
                s
            }, &mut |sq: &mut Squad, _: &mut ()| {
                let index = |k: Option<u32>| k.map(|k| k as usize);
                squad_swipes(sq, true, dt, index(sq.slots[0]), index(sq.slots[1]), &g, &head_on, head_on_dist, &mut ovr);
            });
            // (the common update F-63286 then brakes the cop the target's probe hit, +88: the recorded exit's)
            if let Ok(k) = usize::try_from(u[12].parse::<i32>().unwrap()) {
                ovr[k].brake = Some(1.0);
            }
            let code = |h: bool| if h { 2 } else { 0 };
            let sl = |k: Option<u32>| k.map_or(-1, |k| k as i32);
            let now = code(sq.halt);
            let mut got = vec![now.to_string(), sq.next.map_or(-1, code).to_string()];
            let mut want: Vec<String> = w[1..3].iter().map(|s| s.to_string()).collect();
            got.extend(sq.slots.iter().map(|&k| sl(k).to_string()));
            want.extend(w[7..10].iter().map(|s| s.to_string()));
            if now == 0 && w[1] == "0" {
                got.push(format!("{:08X} {:08X} {:08X} {:02X}", sq.lock.to_bits(), sq.hyst.to_bits(), sq.swipe_t.to_bits(), sq.latch as u8));
                want.push(w[3..7].join(" "));
                for (k, c) in cops.iter().enumerate() {
                    let word = |v: Option<f32>, i: usize| v.map_or(hex(c[i]), f32::to_bits);
                    got.push(format!("{} {} {:08X} {:08X} {:08X}", brains[k].0, brains[k].1 as u8, word(ovr[k].steer, 20), word(ovr[k].brake, 21),
                                     word(ovr[k].handbrake.map(|_| 1.0), 22)));
                    want.push(w[10 + 5 * k..15 + 5 * k].join(" "));
                    swiped += ovr[k].brake.is_some() as u32;
                }
                catchup += 1;
                sends += sent as u32;
            }
            assert_eq!(got, want, "update {checked}");
            checked += 1;
        }
        assert!(checked == 2996 && catchup == 2114 && sends == 9 && swiped == 251, "{checked} {catchup} {sends} {swiped}");
    }

    /// The squad formations against the original's (the Arrow Trap's F-80532, the Double Pursuit's F-21534, with the
    /// enters F-81752 / F-86874) run offline by tools/formation_reference.py: the same inputs from the same xorshift, every
    /// call's send, hysteresis bits, slots and brains equal.
    #[test]
    fn squad_formation_matches_the_original() {
        struct X(u32);
        impl X {
            fn next(&mut self) -> u32 {
                self.0 ^= self.0 << 13;
                self.0 ^= self.0 >> 17;
                self.0 ^= self.0 << 5;
                self.0
            }
            fn r(&mut self, a: i32, b: i32) -> i32 {
                a + (self.next() % (b - a + 1) as u32) as i32
            }
        }
        let reference = include_str!("testdata/formation_ref.txt");
        let mut lines = reference.lines();
        let rows: Vec<f32> = lines.next().unwrap().split_whitespace().skip(1).map(|h| f32::from_bits(u32::from_str_radix(h, 16).unwrap())).collect();
        let e = |v: i32| v as f32 * 0.125;
        let (mut checked, mut sends) = (0, 0);
        while let Some(head) = lines.next() {
            let seed: u32 = head.strip_prefix("CASE ").unwrap().parse().unwrap();
            let mut g = X(seed);
            let arrow = seed % 2 == 0;
            let n = if arrow { 3 } else { 2 };
            let mut t = [g.r(-800, 800), g.r(0, 80), g.r(-800, 800)];
            let mut yaw = g.r(0, 15) as usize;
            let mut cops: Vec<[i32; 3]> = (0..n).map(|_| [t[0] + g.r(-320, 320), t[1] + g.r(-8, 8), t[2] + g.r(-320, 320)]).collect();
            let mut brains: Vec<(u8, bool)> = (0..n).map(|_| ([0, 4, 5][g.r(0, 2) as usize], g.r(0, 1) != 0)).collect();
            let (mut hyst, mut slots) = (0.0f32, [None; 3]);
            for k in 0..150 {
                let enter = k == 0 || g.next() % 128 == 0;
                let dt = if g.next() % 8 == 0 { (1.0f64 / 30.0) as f32 } else { (1.0f64 / 60.0) as f32 };
                for v in &mut t {
                    *v += g.r(-4, 4);
                }
                if g.next() % 32 == 0 {
                    yaw = g.r(0, 15) as usize;
                }
                let mut route = vec![None; n];
                for c in 0..n {
                    for v in &mut cops[c] {
                        *v += g.r(-6, 6);
                    }
                    if g.next() % 5 != 0 {
                        let q = [cops[c][0] + g.r(-40, 40), cops[c][1] + g.r(-40, 40), cops[c][2] + g.r(-40, 40)];
                        route[c] = Some(Vec3::new(e(q[0]), e(q[1]), e(q[2])));
                    }
                    if g.next() % 64 == 0 {
                        let b = ([0, 4, 5][g.r(0, 2) as usize], g.r(0, 1) != 0);
                        if k > 0 {
                            brains[c] = b;
                        }
                    }
                }
                let pos: Vec<Vec3> = cops.iter().map(|c| Vec3::new(e(c[0]), e(c[1]), e(c[2]))).collect();
                let w = &rows[yaw * 4..yaw * 4 + 4];
                let (right, back, tpos) = (Vec3::new(w[0], 0.0, w[1]), Vec3::new(w[2], 0.0, w[3]), Vec3::new(e(t[0]), e(t[1]), e(t[2])));
                let sent = if enter {
                    hyst = 2.0;
                    slots = [None; 3];
                    squad_formation(&mut hyst, &mut slots, arrow, &mut brains, &pos, &route, tpos, right, back, 3.5, 0.0, true);
                    2
                } else {
                    squad_formation(&mut hyst, &mut slots, arrow, &mut brains, &pos, &route, tpos, right, back, 3.5, dt, false) as i32
                };
                let mut got = format!("{} {:08x}", sent, hyst.to_bits());
                for s in slots {
                    got += &format!(" {}", s.map_or(-1, |s| s as i32));
                }
                for b in &brains {
                    got += &format!(" {} {}", b.0, b.1 as u8);
                }
                assert_eq!(got, lines.next().unwrap(), "case {seed} call {k}");
                checked += 1;
                sends += (sent == 1) as u32;
            }
        }
        assert!(checked == 6000 && sends > 20, "{checked} {sends}");
    }

    /// The Pursuit's machine against the original's tactic object (ctor F-55508, begin F-89627, update F-68032 with its
    /// FSM and states) run offline by tools/pursuit_fsm_reference.py: the same inputs from the same xorshift; at every
    /// recorded update the state, the asked state, the state's time, its timers and the brain's command fields equal.
    #[test]
    fn pursuit_machine_matches_the_original() {
        struct X(u32);
        impl X {
            fn next(&mut self) -> u32 {
                self.0 ^= self.0 << 13;
                self.0 ^= self.0 >> 17;
                self.0 ^= self.0 << 5;
                self.0
            }
            fn r(&mut self, a: i32, b: i32) -> i32 {
                a + (self.next() % (b - a + 1) as u32) as i32
            }
        }
        const ODDS: [u32; 12] = [300, 150, 60, 300, 300, 300, 30, 30, 40, 40, 30, 60];
        let reference = include_str!("testdata/pursuit_fsm_ref.txt");
        let mut lines = reference.lines().peekable();
        let (mut checked, mut blocks) = (0, 0);
        while let Some(head) = lines.next() {
            let seed: u32 = head.strip_prefix("CASE ").unwrap().parse().unwrap();
            let mut g = X(seed);
            let mut f: Vec<bool> = (0..12).map(|_| g.r(0, 1) != 0).collect();
            let (mut bearing, mut speed) = (g.r(60, 120), g.r(0, 80));
            let (mut tac, mut last) = (CopTac::default(), 0.0f32);
            let mut k = 0usize;
            while let Some(line) = lines.peek().filter(|l| !l.starts_with("CASE")) {
                let v: Vec<&str> = line.split_whitespace().collect();
                let want: usize = v[0].parse().unwrap();
                while k <= want {
                    for (i, b) in f.iter_mut().enumerate() {
                        if g.next() % ODDS[i] == 0 {
                            *b = !*b;
                        }
                    }
                    bearing = (bearing + g.r(-3, 3)).clamp(50, 130);
                    speed = (speed + g.r(-2, 2)).clamp(0, 120);
                    if g.next() % 97 == 0 {
                        speed = (speed - g.r(10, 40)).max(0);
                    }
                    let s = PursuitSees {
                        near_safe: f[0],
                        near_along: f[1],
                        near5: f[2],
                        moving: f[3],
                        same_dir: f[4],
                        ahead: f[5],
                        on_right: f[6],
                        same_facing: f[7],
                        all_block: f[8],
                        closing: f[9],
                        pick: f[10],
                        gate: f[11],
                        bearing: bearing as f32,
                        speed: speed as f32 * 0.25,
                    };
                    pursuit_machine(&mut tac, &s, 10.0, last, 1.0f32 / 60.0);
                    last = s.speed;
                    k += 1;
                }
                let code = |t: Tac| match t {
                    Tac::CatchUp => 1,
                    Tac::AlongSide => 2,
                    Tac::Block => 3,
                    Tac::Halt => 5,
                    Tac::Avoid => 9,
                };
                let a = match tac.st {
                    Tac::AlongSide => tac.far_t.to_bits(),
                    Tac::Halt => tac.halt_steer.to_bits(),
                    _ => 0,
                };
                let z = if tac.st == Tac::AlongSide { tac.side_t.to_bits() } else { 0 };
                let got = format!("{} {} {} {:08x} {:08x} {:08x} {} {} {}", want, code(tac.st), tac.next.map_or(-1, code), tac.t.to_bits(), a, z, tac.cmd,
                                  tac.side as u8, tac.hb as u8);
                assert_eq!(got, *line, "case {seed}");
                checked += 1;
                blocks += (tac.st == Tac::Block) as u32;
                lines.next();
            }
        }
        assert!(checked > 1000 && blocks > 0, "{checked} {blocks}");
    }

    /// The arrest timers against the original's Pursuit tactic object run offline by tools/arrest_reference.py (F-68032's
    /// gate with the arrest test F-36806, the FSM's Delay exit F-25831, the common update F-63286 with the seen update
    /// F-74492, the +108 update F-29906, the cop query F-29443 / F-39842 and the +140 / +144 timers): the same inputs from the
    /// same xorshift (the generator mirrors `gen_case` there); at every update the test's result, and at every recorded one
    /// the bits of +108, +140, +144 and the seen timers +124 / +132 equal.
    #[test]
    fn evaluate_police_by_the_launchers_rule() {
        let mut v = Vehicles::default();
        let mut p = Police::new();
        // chance 0: always destroyed, the streak untouched
        for _ in 0..10 {
            p.evaluate(PoliceEval::NEVER, 2, 3, true, &mut v);
            assert!(p.race_off && p.units.is_empty() && p.no_cop_races == 0);
        }
        // no zones: destroyed whatever the chance; a chance above 0 counts the streak
        p.evaluate(PoliceEval::SH_04B, 2, 0, true, &mut v);
        assert!(p.race_off && p.no_cop_races == 1);
        // the gate: more than 5 racers - chance 0, the streak not counted
        p.evaluate(PoliceEval::GATED, 6, 3, true, &mut v);
        assert!(p.race_off && p.no_cop_races == 1);
        // chance 1 with zones: the three units, the streak back to 0, SH_04B's chase properties and type
        p.evaluate(PoliceEval::SH_04B, 2, 3, true, &mut v);
        assert!(!p.race_off && p.units.len() == 3 && p.no_cop_races == 0);
        assert!(p.chase_props == [false, false, true] && p.chase_type == 1);
        p.race_left();
        assert!(p.chase_props == [true; 3] && p.chase_type == 0);
        // the streak: after four lost 25 % rolls the fifth race has police
        let mut lost = 0;
        for _ in 0..200 {
            p.evaluate(PoliceEval::PHONE, 2, 3, true, &mut v);
            if p.race_off {
                lost += 1;
                assert!(p.no_cop_races <= 4);
            } else {
                assert_eq!(p.no_cop_races, 0);
            }
        }
        assert!(lost > 0);
    }

    #[test]
    fn arrest_timers_match_the_original() {
        let (cases, ticks, fires, compared, cop_in_ticks, timer_ticks) = arrest_run(include_str!("testdata/arrest_ref.txt"), false);
        assert!(cases == 40 && compared > 1500 && fires > 8 && cop_in_ticks > 3000 && timer_ticks > 3000, "{cases} {ticks} {fires} {compared} {cop_in_ticks} {timer_ticks}");
    }

    /// The same timers in the pull-over's and the arrow trap's classes (tools/arrest_reference.py, second run): their vt+32 /
    /// +40 / +44 / +64 slots (F-66453, F-41284, F-33778, F-82134, F-77220) over the same common update F-63286, the seen
    /// update and the arrest test called in order, the state set directly (the Search of the arrow trap is state 3; the
    /// pull-over's slots 40 / 44 are false); the test's result is compared when the target is not lost.
    #[test]
    fn arrest_timers_match_the_original_in_the_other_classes() {
        let (cases, ticks, fires, compared, cop_in_ticks, timer_ticks) = arrest_run(include_str!("testdata/arrest_vt_ref.txt"), true);
        assert!(cases == 40 && compared > 1500 && fires > 8 && cop_in_ticks > 3000 && timer_ticks > 3000, "{cases} {ticks} {fires} {compared} {cop_in_ticks} {timer_ticks}");
    }

    fn arrest_run(reference: &str, vt: bool) -> (u32, u32, u32, u32, u32, u32) {
        struct X(u32);
        impl X {
            fn next(&mut self) -> u32 {
                self.0 ^= self.0 << 13;
                self.0 ^= self.0 >> 17;
                self.0 ^= self.0 << 5;
                self.0
            }
            fn r(&mut self, a: i32, b: i32) -> i32 {
                a + (self.next() % (b - a + 1) as u32) as i32
            }
        }
        struct Cop {
            o: [i32; 3],
            ug: bool,
            wr: i32,
            br: bool,
            s1: bool,
            sb: bool,
        }
        const DT_BITS: [u32; 4] = [0x3C88_8889, 0x3D08_8889, 0x3C08_8889, 0x3C4A_11C1];
        const SAFE: [i32; 4] = [30, 15, 30, 45];
        const ARREST_T: [f32; 4] = [0.5, 1.0, 1.0, 2.5];
        const THR: [i32; 4] = [5, 10, 10, 20];
        const BIKER: [i32; 4] = [50, 50, 20, 70];
        let allowed = |s: usize| -> &'static [i32] {
            match s {
                0 => &[1],
                1 => &[2, 5, 6],
                2 => &[1, 3, 5, 6],
                3 => &[1, 6],
                5 => &[1, 6],
                _ => &[1],
            }
        };
        let dwell = |s: usize| -> u32 { [40, 120, 90, 70, 1, 90, 150][s] };
        let mut lines = reference.lines().peekable();
        let (mut cases, mut ticks, mut fires, mut compared, mut cop_in_ticks, mut timer_ticks, mut fires_n) = (0, 0, 0, 0, 0, 0, 0);
        while let Some(head) = lines.next() {
            let mut hw = head.strip_prefix("CASE ").unwrap().split_whitespace();
            let seed: u32 = hw.next().unwrap().parse().unwrap();
            let cls: u32 = hw.next().map_or(2, |v| v.parse().unwrap());
            let mut last_fire = false;
            let mut g = X(seed);
            let dt = f32::from_bits(DT_BITS[g.r(0, 3) as usize]);
            let safe = SAFE[g.r(0, 3) as usize] as f32;
            let at = ARREST_T[g.r(0, 3) as usize];
            let thr = THR[g.r(0, 3) as usize] as f32;
            let _biker = BIKER[g.r(0, 3) as usize];
            let n = g.r(1, 3) as usize;
            let bike = g.next() % 3 == 0;
            let smax = g.r(2, 5);
            let mut t = [g.r(-300, 300) * 8, g.r(0, 20) * 8, g.r(-300, 300) * 8];
            if seed % 5 == 0 {
                t = t.map(|v: i32| v.rem_euclid(5) - 2);
            }
            let mut spd = g.r(0, 120);
            let mut go = g.r(0, 1) != 0;
            let (mut cur, mut lostb, mut seen) = (0usize, false, g.r(0, 1) != 0);
            let (mut burn, mut wreck, mut bdown) = (0, 0, false);
            let lim = (safe as i32 + 20) * 16;
            let mut cops: Vec<Cop> = Vec::new();
            for _ in 0..n {
                let o = [g.r(-20, 20) * 16, g.r(-20, 20) * 16, g.r(-20, 20) * 16];
                let (s1, sb) = (g.r(0, 1) != 0, g.r(0, 1) != 0);
                cops.push(Cop { o, ug: false, wr: 0, br: true, s1, sb });
            }
            let mut p = Police::new();
            p.reset_arrest();
            let mut fired = false;
            for k in 0..1200usize {
                let mut ask = -1;
                if g.next() % dwell(cur) == 0 {
                    let o = allowed(cur);
                    ask = o[g.next() as usize % o.len()];
                }
                let post = if ask != -1 { ask as usize } else { cur };
                if post == 6 && ask == -1 {
                    if g.next() % 25 == 0 {
                        lostb = !lostb;
                    }
                } else {
                    lostb = false;
                }
                if g.next() % 12 == 0 {
                    seen = !seen;
                }
                if g.next() % 40 == 0 {
                    go = !go;
                }
                if go {
                    t = [t[0] + g.r(-smax, smax), t[1] + g.r(-smax, smax), t[2] + g.r(-smax, smax)];
                }
                spd = (spd + g.r(-3, 3)).clamp(0, 280);
                if g.next() % 30 == 0 {
                    burn = g.r(0, 2);
                }
                if g.next() % 500 == 0 {
                    wreck = g.r(0, 2);
                }
                if g.next() % 60 == 0 {
                    bdown = !bdown;
                }
                for (ci, c) in cops.iter_mut().enumerate() {
                    for i in 0..3 {
                        c.o[i] = (c.o[i] + g.r(-8, 8)).clamp(-lim, lim);
                    }
                    if g.next() % 90 == 0 {
                        let rr = [safe as i32 * 16, safe as i32 * 16 + 1, safe as i32 * 16 - 1, (safe as i32 + 1) * 16][g.r(0, 3) as usize];
                        let ax = g.r(0, 2) as usize;
                        let sg = if g.next() % 2 == 0 { 1 } else { -1 };
                        c.o = [0, 0, 0];
                        c.o[ax] = rr * sg;
                    }
                    if g.next() % 200 == 0 {
                        c.ug = !c.ug;
                    }
                    if g.next() % 300 == 0 {
                        c.wr = g.r(0, 2);
                    }
                    if g.next() % 500 == 0 && ci > 0 {
                        c.br = !c.br;
                    }
                    if g.next() % 15 == 0 {
                        c.s1 = !c.s1;
                    }
                    if g.next() % 15 == 0 {
                        c.sb = !c.sb;
                    }
                }
                // ---- the port, on the same inputs
                let tp = Vec3::new(t[0] as f32 / 16.0, t[1] as f32 / 16.0, t[2] as f32 / 16.0);
                let cop_in = cops.iter().any(|c| {
                    let cp = Vec3::new((t[0] + c.o[0]) as f32 / 16.0, (t[1] + c.o[1]) as f32 / 16.0, (t[2] + c.o[2]) as f32 / 16.0);
                    !c.ug && c.wr == 0 && c.br && within_sq(tp, cp, safe)
                });
                let a = ArrestIn {
                    seer_r1: cops.iter().any(|c| c.s1),
                    seer_bike: cops.iter().any(|c| c.sb),
                    delay: !vt && (cur == 0 || ask != -1),
                    leaving: !vt && cur == 0 && ask != -1,
                    search: post == 6 && (!vt || cls == 1),
                    lost: lostb && (!vt || cls == 1),
                    cop_in,
                    refresh: cops[0].br,
                    slow: spd as f32 / 16.0 < arrest_ms(thr),
                    bike_down: bike && bdown,
                    wrecked: wreck >= 1,
                    burning: burn == 1,
                    pos: tp,
                };
                let lost_now = a.lost;
                p.seen_update(seen, dt);
                let fire = p.arrest_step(a, safe, at, 0.0, arrest_ms(thr) * dt, dt);
                // ---- the reference
                let row = lines.peek().filter(|l| !l.starts_with("CASE")).map(|l| l.split_whitespace().collect::<Vec<_>>()).filter(|v| v[0].parse::<usize>().unwrap() == k);
                if let Some(v) = &row {
                    last_fire = v[3] == "1";
                }
                let want_fire = if vt { last_fire } else { row.is_some() && last_fire };
                if !(vt && lost_now) {
                    assert_eq!(fire, want_fire, "case {seed} update {k}: the arrest test");
                }
                if let Some(v) = row {
                    if vt || !fire {
                        let (c1, c2) = if vt { (v[1].parse().unwrap(), v[2].parse().unwrap()) } else { (cur, post) };
                        let got = format!("{} {} {} {:08x} {:08x} {:08x} {:08x} {:08x}", k, c1, c2, p.arrest.to_bits(), p.t_burn.to_bits(), p.t_still.to_bits(), p.t_seen.to_bits(), p.t_unseen.to_bits());
                        let want = format!("{} {} {} {} {} {} {} {}", v[0], v[1], v[2], v[4], v[5], v[6], v[7], v[8]);
                        assert_eq!(got, want, "case {seed} update {k}");
                        compared += 1;
                    }
                    lines.next();
                }
                ticks += 1;
                cop_in_ticks += cop_in as u32;
                timer_ticks += (p.arrest > 0.0 || p.t_burn > 0.0 || p.t_still > 0.0) as u32;
                fires_n += (vt && fire) as u32;
                if fire && !vt {
                    fired = true;
                    break;
                }
                cur = post;
            }
            assert!(lines.peek().map_or(true, |l| l.starts_with("CASE")), "case {seed}: reference rows left over");
            cases += 1;
            fires += fired as u32;
        }
        (cases, ticks, fires + fires_n, compared, cop_in_ticks, timer_ticks)
    }

    /// The original's pull-over tactic object (F-36876 with its FSM and states) run offline
    /// (tools/pullover_fsm_reference.py) against `pullover_machine` on the same inputs: at every recorded update the state,
    /// the asked state, the state's time, AvoidCollision's timer / Halt's squared distance and growing time, the brain's
    /// command fields and the velocity factor equal.
    #[test]
    fn brain_override_matches_the_original() {
        // the original's Pursuit / AlongSide / Stopping ticks run offline (tools/brain_override_reference.py)
        let txt = include_str!("testdata/brain_override_ref.txt");
        let (mut n, mut bad) = (0, 0);
        for line in txt.lines() {
            let (inp, out) = line.split_once(" | ").unwrap();
            let v: Vec<&str> = inp.split_whitespace().collect();
            let i = |k: usize| v[k].parse::<i32>().unwrap();
            let h = |k: usize| u32::from_str_radix(v[k], 16).unwrap();
            let fl = h(0);
            let q = |x: i32| x as f32 * 0.25;
            let (cx, cy, cz) = (q(i(3)), q(i(4)), q(i(5)));
            let (tx, ty, tz) = (q(i(3) + i(6)), q(i(4) + i(7)), q(i(5) + i(8)));
            let (dx, dy, dz) = (tx - cx, ty - cy, tz - cz);
            let s = BrainSees {
                moving: fl & 1 != 0,
                ahead: fl & 2 != 0,
                same_dir: fl & 4 != 0,
                ed78: fl & 8 != 0,
                cspeed: q(i(1)),
                tspeed: q(i(2)),
                d2: fmas(dx, dx, fmas(dz, dz, dy * dy)),
                safe: 30.0,
            };
            let stop = StopParams { hb: v[10] != "0", amount: f32::from_bits(h(11)), throttle: f32::from_bits(h(12)), steer: f32::from_bits(h(13)) };
            let side = f32::from_bits(h(9));
            let got: Vec<String> = [4u8, 5, 1]
                .iter()
                .map(|&c| brain_override(c, side, &stop, &s).iter().map(|f| format!("{:08x}", f.to_bits())).collect::<Vec<_>>().join(" "))
                .collect();
            n += 1;
            if got.join(" | ") != out {
                bad += 1;
                if bad < 5 {
                    eprintln!("{line}\n  port {}", got.join(" | "));
                }
            }
        }
        assert!(n > 3000, "{n}");
        assert_eq!(bad, 0, "{bad} of {n} differ");
    }

    #[test]
    fn pullover_machine_matches_the_original() {
        struct X(u32);
        impl X {
            fn next(&mut self) -> u32 {
                self.0 ^= self.0 << 13;
                self.0 ^= self.0 >> 17;
                self.0 ^= self.0 << 5;
                self.0
            }
            fn r(&mut self, a: i32, b: i32) -> i32 {
                a + (self.next() % (b - a + 1) as u32) as i32
            }
        }
        const ODDS: [u32; 9] = [200, 150, 60, 60, 120, 150, 200, 30, 60];
        let reference = include_str!("testdata/pullover_fsm_ref.txt");
        let mut lines = reference.lines().peekable();
        let (mut checked, mut halts) = (0, 0);
        while let Some(head) = lines.next() {
            let seed: u32 = head.strip_prefix("CASE ").unwrap().parse().unwrap();
            let mut g = X(seed);
            let mut f: Vec<bool> = (0..9).map(|_| g.r(0, 1) != 0).collect();
            let (mut speed, mut x, mut z) = (g.r(0, 60), g.r(-200, 200), g.r(-200, 200));
            let mut tac = CopTac::default();
            let mut factor = 1.0f32;
            let mut k = 0usize;
            while let Some(line) = lines.peek().filter(|l| !l.starts_with("CASE")) {
                let v: Vec<&str> = line.split_whitespace().collect();
                let want: usize = v[0].parse().unwrap();
                while k <= want {
                    for (i, b) in f.iter_mut().enumerate() {
                        if g.next() % ODDS[i] == 0 {
                            *b = !*b;
                        }
                    }
                    speed = (speed + g.r(-2, 2)).clamp(0, 60);
                    x = (x + g.r(-3, 3)).clamp(-400, 400);
                    z = (z + g.r(-3, 3)).clamp(-400, 400);
                    let (dx, dz) = (x as f32 * 0.25, z as f32 * 0.25);
                    let s = PullSees {
                        all_halt: f[0],
                        moving: f[1],
                        closing0: f[2],
                        closing7: f[3],
                        closing50: f[4],
                        near50: f[5],
                        near_pull: f[6],
                        pick: f[7],
                        gate: f[8],
                        cspeed: speed as f32 * 0.25,
                        d2: fmas(dx, dx, fmas(dz, dz, 0.0f32 * 0.0)),
                    };
                    factor = pullover_machine(&mut tac, &s, 1.0f32 / 60.0);
                    k += 1;
                }
                let code = |t: Tac| match t {
                    Tac::CatchUp => 0,
                    Tac::Avoid => 1,
                    Tac::Halt => 2,
                    _ => 9,
                };
                let a = match tac.st {
                    Tac::Avoid => tac.side_t.to_bits(),
                    Tac::Halt => tac.halt_d2.to_bits(),
                    _ => 0,
                };
                let w = if tac.st == Tac::Halt { tac.grow_t.to_bits() } else { 0 };
                let got = format!("{} {} {} {:08x} {:08x} {:08x} {} {} {} {:08x}", want, code(tac.st), tac.next.map_or(-1, code), tac.t.to_bits(), a, w,
                                  tac.cmd, tac.side as u8, tac.hb as u8, factor.to_bits());
                assert_eq!(got, *line, "case {seed}");
                checked += 1;
                halts += (tac.st == Tac::Halt) as u32;
                lines.next();
            }
        }
        assert!(checked > 1000 && halts > 0, "{checked} {halts}");
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
        // plus a soft cruiser hit (0x40, entry 6) and a prop (entry 4)
        p.offences |= LAW_POLICE_VEHICLE | LAW_PROP;
        assert_eq!(p.law_fines(), 120.0 + 160.0 + 140.0);
        // a hard one (0x20 + 0x40: entry 5, the 0x40 entry cancels)
        p.offences |= LAW_POLICE_HARD;
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
        // the NO FINES cheat: 0, not the floor
        q.no_charges = true;
        assert_eq!(q.law_fines(), 0.0);
    }
}

/// The arrest's scene (CopArrest.sco 0xc33 - 0xeef): one cop in the latest tactic -> `cut_cop_arrest` or `_07`
/// (RAND_INT_RANGE(0, 1), upper bound included: F-32828), more -> `_02`, `_03` or `_08` (RAND_INT_RANGE(0, 2)).
/// `r` stands in for the game's random number. (The payback's goons scenes and the wiped-out biker's
/// `cut_cop_arrest_bikestand` are not ported.)
pub fn arrest_scene(cops: usize, r: u32, bike_down: bool, goons: bool) -> &'static str {
    // (CopArrest.sco 0xad3: PoliceManager_PlayWipedOutBikerScene F-60341 - the manager's +64, set by F-22530 when the
    // arrested target is a motorcycle wiped out (+592 bit 0x10) - before the cop count; 0xc33: the payback (race type 5)
    // RAND_INT_RANGE(0, 1) == 1 -> _01)
    if bike_down {
        return "cut_cop_arrest_bikestand";
    }
    if goons {
        return ["payback_goons_walkup_02", "payback_goons_walkup_01"][(r % 2) as usize];
    }
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
/// What the arrest timer reads of the target and the tactic's cops this frame.
#[derive(Clone, Copy)]
struct ArrestIn {
    /// a tactic cop sees the target within the tactic's radius + 1 m (F-43276 -> F-94676: in range, a clear line 2 m
    /// above both cars, no cone)
    seer_r1: bool,
    /// the same within fBikerWipeoutArrestDist
    seer_bike: bool,
    /// no arrest test this update: the Pursuit's Delay is running or a state is asked (F-68032's gate)
    delay: bool,
    /// the Delay's exit runs this update, before the timers (F-25831 zeroes +108, +140, +144)
    leaving: bool,
    /// the Search state (vt+40, F-96991): +140 / +144 zeroed
    search: bool,
    /// the Search's clock +16 is above 0: the target is lost (vt+44, F-77220) - no +108 step, no arrest
    lost: bool,
    /// some tactic cop not damaged out has its car strictly within the radius of the target (F-29443)
    cop_in: bool,
    /// the first cop has a brain with a target: the target's place +48 is refreshed
    refresh: bool,
    slow: bool,
    bike_down: bool,
    /// damaged out (F-12032: damage >= MaxDamage)
    wrecked: bool,
    /// burnout charge up (chassis +580, F-25339)
    burning: bool,
    pos: Vec3,
}

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

/// The arrest scenes whose t 0 event turns the player's engine off (car_off on slot 0, or engine_off in _07; F-91263).
pub fn arrest_engine_off(scene: &str) -> bool {
    matches!(scene.rsplit('/').next().unwrap_or(scene).to_ascii_lowercase().as_str(), "cut_cop_arrest" | "cut_cop_arrest_02" | "cut_cop_arrest_03" | "cut_cop_arrest_07" | "cut_cop_arrest_08" | "cut_cop_arrest_bikestand")
}

/// The arrest scene's `fade_to_black` event (F-91263: duration <= 0 -> 1.0 s; every arrest scene's is 1.0 s): its start
/// in the scene's seconds, from the generated scene scripts' event records. The cut_cop_walk_* scenes have none.
pub fn arrest_fade(scene: &str) -> Option<f32> {
    Some(match scene.rsplit('/').next().unwrap_or(scene).to_ascii_lowercase().as_str() {
        "cut_cop_arrest" => 3.0,
        "cut_cop_arrest_02" => 5.0,
        "cut_cop_arrest_02_guns" => 8.333,
        "cut_cop_arrest_03" => 4.0,
        "cut_cop_arrest_04" => 10.0,
        "cut_cop_arrest_05" => 9.0,
        "cut_cop_arrest_06" => 5.667,
        "cut_cop_arrest_07" => 4.0,
        "cut_cop_arrest_08" => 5.667,
        "cut_cop_arrest_bikestand" => 5.0,
        "payback_goons_walkup_01" => 7.667,
        "payback_goons_walkup_02" => 8.333,
        _ => return None,
    })
}

/// OpponentManager_HideOpponents (F-55817 -> F-41633), CopArrest.sco 0x6d0 under the held black: with a race current
/// every race entry i is put on lot slot i (7 slots), heading 0, at once (F-15210 -> F-15222), reset (F-89258) and
/// deactivated (F-12981). Nothing undoes it (no ShowOpponents native). Entry 0 is the player (F-41633, the race data
/// block): moved with `player` - the port's payback, the one race still current here; its scene is then placed from
/// the lot. (The player is not marked deactivated: the arrest's hold keeps his control until Racer_Activate.) The
/// port's other entries are its pinned cars in order.
pub fn hide_opponents(vehicles: &mut Vehicles, world: &World, player: bool) {
    const LOT: [(f32, f32); 7] = [(-520.0, -390.0), (-510.0, -390.0), (-500.0, -390.0), (-490.0, -390.0), (-480.0, -390.0), (-520.0, -380.0), (-510.0, -380.0)];
    let mut i = 1;
    for c in vehicles.cars.iter_mut().filter(|c| (player && c.is_player) || (c.pinned && !c.is_player && !c.cop && !c.goon)) {
        let k = if c.is_player { 0 } else { i };
        let Some(&(x, z)) = LOT.get(k) else { continue };
        if !c.is_player {
            i += 1;
        }
        let at = Vec3::new(x, 9.5, z);
        let pos = world.collider.ground(at, 1.0, 3.0).map_or(at, |h| Vec3::new(at.x, h.point.y, at.z));
        // (heading 0, the station reset's convention: forward (sin h, 0, cos h))
        let q = glam::Quat::from_rotation_y(0f32.atan2(-1.0));
        if let Some(vt) = c.params().veh.clone() {
            crate::physics::vehsim::set_pose(&mut c.state, &vt, &world.collider, [q * Vec3::X, q * Vec3::Y, q * Vec3::Z, pos], Vec3::ZERO, Vec3::ZERO);
        } else {
            c.state.pos = pos;
            c.state.rot = q;
            c.state.vel = Vec3::ZERO;
            c.state.ang_vel = Vec3::ZERO;
        }
        c.pose_history.reset();
        // (Racer Reset, F-89258: +676 = 0 and the driver instance's reset - its scene show flag off, cinedriver.rs)
        c.show_driver = false;
        c.deactivated = !c.is_player;
        log::info!("police: HideOpponents - race entry {} (uid {}) to the lot {:?}", k, c.uid, pos);
    }
}

/// The path point nearest `p`.
fn nearest_point(path: &[Vec3], p: Vec3) -> usize {
    path.iter().enumerate().min_by(|a, b| a.1.distance_squared(p).total_cmp(&b.1.distance_squared(p))).map_or(0, |(k, _)| k)
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

/// The arrest scene's gesture on slot 0 (the player's driver): (gesture, start, duration) - the cops' hands up, the
/// payback goons' walk-ups' `gesture_listen_left` (_01 at 0 for 8.6667 s, _02 at 0.3333 for 9 s).
pub fn arrest_gesture(scene: &str) -> Option<(crate::drivers::Gesture, f32, f32)> {
    use crate::drivers::Gesture;
    match scene.rsplit('/').next().unwrap_or(scene).to_ascii_lowercase().as_str() {
        "payback_goons_walkup_01" => Some((Gesture::ListenLeft, 0.0, 8.6667)),
        "payback_goons_walkup_02" => Some((Gesture::ListenLeft, 0.3333, 9.0)),
        _ => arrest_hands_up(scene).map(|(t, d)| (Gesture::CopHandsUp, t, d)),
    }
}

/// CopArrest.sco after the ARRESTED form (0x1178): Racer_InitializeResetPosition(player, (-604.5, 7, -340.5), 5 deg) and
/// ResetWithDamageIntact (F-39106: the place probed from 1 m above to 3 m below, F-15222) - the police station, upright,
/// standing, the damage kept; PoliceManager_CutSceneDone (F-30655) then sets a damaged-out target's damage to 0.95 of its
/// MaxDamage. `n` = frames waited so far for the station's ground; Some(n + 1) while it has not streamed in (120 at most).
pub fn station_reset(vehicles: &mut Vehicles, world: &World, n: u32) -> Option<u32> {
    let c = vehicles.player_mut()?;
    let at = Vec3::new(-604.5, 7.0, -340.5);
    // (5 deg is the car's yaw, as every Racer_InitializeResetPosition heading: it faces (-sin h, 0, -cos h))
    let h = 5f32.to_radians();
    let f = Vec3::new(-h.sin(), 0.0, -h.cos());
    let q = glam::Quat::from_rotation_y((-f.x).atan2(-f.z));
    let g = world.collider.ground(at, 1.0, 3.0);
    let pos = g.map_or(at, |h| Vec3::new(at.x, h.point.y, at.z));
    if let Some(vt) = c.params().veh.clone() {
        crate::physics::vehsim::set_pose(&mut c.state, &vt, &world.collider, [q * Vec3::X, q * Vec3::Y, q * Vec3::Z, pos], Vec3::ZERO, Vec3::ZERO);
    } else {
        c.state.pos = pos;
        c.state.rot = q;
        c.state.vel = Vec3::ZERO;
        c.state.ang_vel = Vec3::ZERO;
    }
    c.pose_history.reset();
    // (ResetWithDamageIntact, F-39106: the scene show flag off, cinedriver.rs)
    c.show_driver = false;
    if n == 0 {
        log::info!("police: after the form - the car at the police station {:?} (damage {:.3})", pos, c.state.damage);
        if damaged_out(c.state.damage) {
            c.state.damage = 0.95;
        }
    }
    (g.is_none() && n < 120).then_some(n + 1)
}

/// The police scenes' place (CopPullover.sco 0x54c, CopArrest.sco 0xa1f): the player's car put upright where it
/// stands with its heading (Racer_InitializeResetPosition, F-40200, and ResetWithDamageIntact, F-67152), and the
/// scene's frame - that heading alone, snapped to the ground from 5 m above to 5 m below and leaned with it when it
/// is off by more than 0.002 rad (SetGameCoordSysBindingCopyRacer, F-94483: F-88556, F-33119); never the car's
/// own pitch or roll. (The nearest scene safezone or road point, FindClosestSceneSafezone, is not ported: the
/// car's own place, the original's "none found".)
pub fn scene_frame(vehicles: &mut Vehicles, world: &World, zones: &[(Vec3, f32)]) -> Option<glam::Mat4> {
    let place = scene_place(vehicles, world, zones)?;
    scene_frame_at(vehicles, world, place)
}

/// The scene's place and heading (degrees), taken where the car stands now: CineScript_FindClosestSceneSafezone, or
/// none found the frame the script filled - the car's place and Racer_GetAngleY's heading (CopPullover.sco 0x129 /
/// CopArrest.sco 0x95d). CopPullover takes it before its 1 s fade down (0x149) and moves the car there after (0x54c).
pub fn scene_place(vehicles: &Vehicles, world: &World, zones: &[(Vec3, f32)]) -> Option<(Vec3, f32)> {
    let c = vehicles.player()?;
    let own = (c.state.pos, crate::brain::racer_angle_y(Some([c.state.rot * Vec3::X, c.state.rot * Vec3::Y, c.state.rot * Vec3::Z])));
    closest_scene_safezone(world, zones, c.state.pos).or(Some(own))
}

/// `scene_frame` at a place `scene_place` gave.
pub fn scene_frame_at(vehicles: &mut Vehicles, world: &World, place: (Vec3, f32)) -> Option<glam::Mat4> {
    let c = vehicles.player_mut()?;
    // (the place and heading replace the car's - Racer_InitializeResetPosition and ResetWithDamageIntact put the car
    // there upright)
    let (pos, f) = match Some(place) {
        Some((p, h)) => {
            let h = h.to_radians();
            log::info!("police: scene place {:?} heading {:.1} (car at {:?})", p, h.to_degrees(), c.state.pos);
            // (the heading is the car's yaw, euler (0, h, 0) - F-40200, F-15222: it faces (-sin h, 0, -cos h); a road
            // point's heading is the reversed lane's angle, so the car faces along the lane's travel, on its side of the road)
            (p, Vec3::new(-h.sin(), 0.0, -h.cos()))
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
    // (ResetWithDamageIntact, F-39106: racer +676 = 0 and the driver instance's reset - its scene show flag off,
    // cinedriver.rs; the car scenes' UseRacerNativeCharacter(0) shows it again, the bikestand's script does not)
    c.show_driver = false;
    let (place, lean) = match world.collider.ground(pos, 5.0, 5.0) {
        Some(h) if h.normal.angle_between(Vec3::Y) > 0.002 => (Vec3::new(pos.x, h.point.y, pos.z), glam::Quat::from_rotation_arc(Vec3::Y, h.normal.normalize())),
        Some(h) => (Vec3::new(pos.x, h.point.y, pos.z), glam::Quat::IDENTITY),
        None => (pos, glam::Quat::IDENTITY),
    };
    Some(glam::Mat4::from_rotation_translation(lean * q, place))
}

/// The offence gate of F-88545 (burn-out, wrong way, block traffic): `cond` this tick, `started` = the offence's started
/// bit (kept to the record's clear). Started: the bit on every tick the condition holds, the timer untouched (it is the
/// per-second carry); not started: the bit once the timer has reached `thr` (compared BEFORE the add), else the timer
/// grows by `dt`. Without the condition the timer is zeroed unless started.
fn offence_gate(cond: bool, started: bool, timer: &mut f32, thr: f32, dt: f32) -> bool {
    if cond {
        if started || *timer >= thr {
            return true;
        }
        *timer += dt;
    } else if !started {
        *timer = 0.0;
    }
    false
}

/// Block traffic (F-60011): over an ai_road polygon and not over an ai_shortcut / ai_sidewalk / ai_grass one (F-66554,
/// F-75972, F-86151); no road node within 3 m (a junction box): slow (under 2.235 m/s) is blocking; else under
/// iBlockTrafficSpeedPrcnt % of the road's limit (the limit 0: fSpeedLimitLow x 0.447), not on the arm that has green or
/// yellow now of the junction ahead (F-65298, F-75755, F-93041: within half its reach + 10 m, the arm at 150 degrees or
/// more from the direction to the junction centre), and an 8 m level probe forward from half the box length ahead and
/// half its height up hits nothing.
fn block_traffic_cond(world: &World, vehicles: &Vehicles, p: &crate::vehicles::CarInstance, peds: &[Vec3], tun: &Tuning) -> bool {
    let (Some(nav), Some(mesh)) = (world.nav.as_ref(), crate::offroad::mesh()) else { return false };
    let pos = p.state.pos;
    let fwd = p.state.forward();
    let vel = p.state.vel;
    let v = vel.length();
    if !mesh.on_road(pos) || mesh.on_flag(pos, 8) || mesh.on_flag(pos, 1) || mesh.on_flag(pos, 4) {
        return false;
    }
    let flat = Vec3::new(pos.x, 0.0, pos.z);
    let Some(NavNode::Road(road)) = nav.node_within(flat, 3.0, false) else { return v < 2.235 };
    if vehicles.driver_lanes.get(&p.uid).map_or(true, |d| d.best.is_none()) {
        return false;
    }
    let limit = nav.zones.get(road).map(|z| z.speed_limit).filter(|l| l.is_finite() && *l > 0.0).unwrap_or(tun.get("fSpeedLimitLow", 50.0) * 0.447);
    if v >= tun.get("iBlockTrafficSpeedPrcnt", 10.0) * 0.01 * limit {
        return false;
    }
    // (F-65298: the road's junction ends, A first; the first one ahead along the facing, or the velocity from 2.235 m/s)
    let dir = if v < 2.235 { fwd } else { vel };
    let ahead = nav
        .next_of(NavNode::Road(road))
        .iter()
        .filter_map(|n| if let NavNode::Junction(j) = n { Some(*j) } else { None })
        .take(2)
        .find(|&j| nav.junction_rects.get(j).map_or(false, |r| (r.c[0] - pos.x) * dir.x + (r.c[1] - pos.z) * dir.z > 0.0));
    // (the navsys junction list has 351 entries, the signals' only those with arms: matched by the centre)
    let sig = ahead.and_then(|j| nav.junction_centers.get(j)).and_then(|c| vehicles.signals.junctions.iter().find(|s| (s.center - *c).length_squared() < 0.01));
    if let Some(j) = sig.filter(|j| !j.arms.is_empty()) {
        let d = pos - j.center;
        let d2 = d.x * d.x + j.center.y * j.center.y + d.z * d.z;
        let reach = 0.5 * j.reach + 10.0;
        if d2 < reach * reach {
            // (green below 6.5, yellow to 8.5, red all round after; J +160 / +164 only, a forced arm or forced red through
            // `step`)
            let arm = running_arm(j.timer, j.current as i32, &j.arms);
            if arm == Vec3::ZERO {
                return false;
            }
            let to = Vec3::new(j.center.x - pos.x, j.center.y, j.center.z - pos.z);
            if arm.dot(to.normalize_or_zero()).clamp(-1.0, 1.0).acos() < 2.61799 {
                return false;
            }
        }
    }
    let ext = p.model.max - p.model.min;
    let s = pos + fwd * (0.5 * ext.z) + p.state.up() * (0.5 * ext.y);
    !level_probe_front(world, vehicles, peds, p.uid, s, fwd, 8.0)
}

/// F-20855 as the seen test and block traffic call it: a one-sided line probe from `o` along `d` for `len`, any hit
/// counts - the city bound, the other cars' hulls, the props (the level's own and the moved / broken ones) and the
/// walkers' capsules (radius 0.3, 0.3 to 1.5 m above the feet).
/// `level_probe_front` and the car whose hull is the nearest hit (F-16046 keeps the nearest hit and looks its instance up in
/// the tactic's cops): the cars' and the city's distances are compared; a prop or a walker hit counts for the hit only
/// (their distances not computed: approximate when one lies between the target and a car).
fn level_probe_nearest_car(world: &World, vehicles: &Vehicles, peds: &[Vec3], skip_uid: u32, o: Vec3, d: Vec3, len: f32) -> (bool, Option<u32>) {
    let city = world.collider.probe_front(o, d, len).map(|h| h.t);
    let car = vehicles.cars.iter().filter(|c| c.uid != skip_uid)
        .filter_map(|c| hull_probe_t(c, o, d, len).map(|t| (t, c.uid)))
        .min_by(|a, b| a.0.total_cmp(&b.0));
    let near = car.filter(|&(t, _)| city.map_or(true, |ct| t < ct)).map(|(_, u)| u);
    let hit = city.is_some() || car.is_some()
        || props_probe(world.collider.loose.as_deref(), &vehicles.level, o, d, len)
        || peds.iter().any(|&q| seg_capsule(o, d, len, q + Vec3::Y * 0.3, q + Vec3::Y * 1.5, 0.3));
    (hit, near)
}

fn level_probe_front(world: &World, vehicles: &Vehicles, peds: &[Vec3], skip_uid: u32, o: Vec3, d: Vec3, len: f32) -> bool {
    world.collider.probe_front(o, d, len).is_some()
        || vehicles.cars.iter().filter(|c| c.uid != skip_uid).any(|c| hull_probe_front(c, o, d, len))
        || props_probe(world.collider.loose.as_deref(), &vehicles.level, o, d, len)
        || peds.iter().any(|&q| seg_capsule(o, d, len, q + Vec3::Y * 0.3, q + Vec3::Y * 1.5, 0.3))
}

/// The segment o -> o + d x len against a capsule (axis a -> b, radius r), entering from outside.
fn seg_capsule(o: Vec3, d: Vec3, len: f32, a: Vec3, b: Vec3, r: f32) -> bool {
    let (p1, q1, p2, q2) = (o, o + d * len, a, b);
    let (d1, d2, rr) = (q1 - p1, q2 - p2, p1 - p2);
    let (aa, ee, f) = (d1.dot(d1), d2.dot(d2), d2.dot(rr));
    let c = d1.dot(rr);
    let bb = d1.dot(d2);
    let den = aa * ee - bb * bb;
    let mut sn = if den > 1e-9 { ((bb * f - c * ee) / den).clamp(0.0, 1.0) } else { 0.0 };
    let mut tn = if ee > 1e-9 { (bb * sn + f) / ee } else { 0.0 };
    if tn < 0.0 {
        tn = 0.0;
        sn = if aa > 1e-9 { (-c / aa).clamp(0.0, 1.0) } else { 0.0 };
    } else if tn > 1.0 {
        tn = 1.0;
        sn = if aa > 1e-9 { ((bb - c) / aa).clamp(0.0, 1.0) } else { 0.0 };
    }
    let close = (p1 + d1 * sn) - (p2 + d2 * tn);
    // (the start inside the solid is no hit: the line enters from outside)
    let start = {
        let t = if ee > 1e-9 { (rr.dot(d2) / ee).clamp(0.0, 1.0) } else { 0.0 };
        (rr - d2 * t).length_squared()
    };
    start > r * r && close.length_squared() <= r * r
}

/// One part of a prop against the probe, in the part's frame `f` (rigid): its bounding sphere first, then the primitive
/// by kind (0 sphere, 1 capsule along local Y, 3 box, 4 polyhedron); a hit must enter from outside.
fn seg_bound(b: &mcla_formats::rage::fragphys::FragBound, f: &[Vec3; 4], o: Vec3, d: Vec3, len: f32) -> bool {
    use crate::physics::loose::{inv, v};
    let (lo, ld) = (inv(f, o - f[3]), inv(f, d));
    let c = v(b.centroid);
    // (the bounding sphere)
    let t = (c - lo).dot(ld).clamp(0.0, len);
    if (lo + ld * t - c).length_squared() > b.radius * b.radius {
        return false;
    }
    match b.kind {
        0 => seg_capsule(lo, ld, len, c, c, b.round),
        1 => {
            let h = Vec3::Y * (0.5 * b.length);
            seg_capsule(lo, ld, len, c - h, c + h, b.round)
        }
        3 | 4 if !b.tris.is_empty() => b.tris.iter().any(|t| {
            let tri = [v(b.verts[t[0] as usize]), v(b.verts[t[1] as usize]), v(b.verts[t[2] as usize])];
            crate::physics::collision::ray_tri_front(lo, ld, &tri, len).is_some()
        }),
        3 => {
            // (a box without faces: the slab test, entering from outside)
            let (mn, mx) = (v(b.min), v(b.max));
            if lo.cmpge(mn).all() && lo.cmple(mx).all() {
                return false;
            }
            let (mut t0, mut t1) = (0.0f32, len);
            for k in 0..3 {
                if ld[k].abs() < 1e-9 {
                    if lo[k] < mn[k] || lo[k] > mx[k] {
                        return false;
                    }
                } else {
                    let (a, bb) = ((mn[k] - lo[k]) / ld[k], (mx[k] - lo[k]) / ld[k]);
                    t0 = t0.max(a.min(bb));
                    t1 = t1.min(a.max(bb));
                }
            }
            t0 <= t1 && t0 < len
        }
        _ => false,
    }
}

/// The props of the level on the probe (F-20855, states 7: every placed prop whatever its state): the instances of the
/// file (at their moved place and with their remaining groups when the level holds them), and the pieces and broken-off
/// groups only the level holds.
fn props_probe(lw: Option<&crate::physics::loose::LooseWorld>, lv: &crate::physics::loose::LooseState, o: Vec3, d: Vec3, len: f32) -> bool {
    use crate::physics::loose::{m4, part_present};
    let Some(lw) = lw else { return false };
    let moved: std::collections::HashMap<u32, &crate::physics::loose::Obj> = lv.objs.iter().filter(|x| x.veh.is_none() && x.amb.is_none()).filter_map(|x| Some((x.inst?, x))).collect();
    let test = |ty: usize, m: &[Vec3; 4], absent: u64| {
        let t = &lw.types[ty];
        t.parts.iter().enumerate().filter(|(c, _)| part_present(t, absent, *c)).any(|(_, p)| match p.bound.as_ref() {
            Some(b) => seg_bound(b, &crate::physics::gjk::world_matrix(&m4(&p.matrix), m), o, d, len),
            None => false,
        })
    };
    lw.near(o + d * (0.5 * len), 0.5 * len + 10.0).into_iter().any(|i| match moved.get(&i) {
        Some(x) => test(x.ty, &x.matrix, x.entry.map_or(0, |e| e.absent)),
        None => test(lw.instances[i as usize].1, &m4(&lw.instances[i as usize].0.matrix), 0),
    }) || lv.objs.iter().filter(|x| x.inst.is_none() && x.veh.is_none() && x.amb.is_none()).any(|x| test(x.ty, &x.matrix, x.entry.map_or(0, |e| e.absent)))
}

/// A one-sided segment o -> o + d x len against a car's hull triangles (its bound's faces seen from outside).
/// The nearest front-face hit of the probe on a car's hull, its distance.
fn hull_probe_t(c: &crate::vehicles::CarInstance, o: Vec3, d: Vec3, len: f32) -> Option<f32> {
    let reach = 0.5 * (c.model.max - c.model.min).length() + len;
    if (c.state.pos - o).length_squared() > reach * reach {
        return None;
    }
    let h = c.model.params.hull.as_ref()?;
    let inv = c.state.rot.inverse();
    let (lo, ld) = (inv * (o - c.state.pos), inv * d);
    h.tris.iter().filter_map(|t| {
        let tri = [h.verts[t[0] as usize], h.verts[t[1] as usize], h.verts[t[2] as usize]];
        crate::physics::collision::ray_tri_front(lo, ld, &tri, len).map(|(t, _)| t)
    }).min_by(|a, b| a.total_cmp(b))
}

fn hull_probe_front(c: &crate::vehicles::CarInstance, o: Vec3, d: Vec3, len: f32) -> bool {
    let reach = 0.5 * (c.model.max - c.model.min).length() + len;
    if (c.state.pos - o).length_squared() > reach * reach {
        return false;
    }
    let Some(h) = c.model.params.hull.as_ref() else { return false };
    let inv = c.state.rot.inverse();
    let (lo, ld) = (inv * (o - c.state.pos), inv * d);
    h.tris.iter().any(|t| {
        let tri = [h.verts[t[0] as usize], h.verts[t[1] as usize], h.verts[t[2] as usize]];
        crate::physics::collision::ray_tri_front(lo, ld, &tri, len).is_some()
    })
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
    fn the_cop_zone_waits_5_5_s_then_the_radio_delay() {
        let dt = 1.0 / 30.0;
        const FAR: Vec3 = Vec3::new(1.0e4, 0.0, 0.0);
        let racers = [(10u32, true, Vec3::ZERO, false), (11, false, Vec3::new(40.0, 0.0, 0.0), false), (12, false, Vec3::new(20.0, 0.0, 0.0), false)];
        // never the AI (chance 0): the player is the target; 5.5 s, then fCopZoneRdoDelayTime 2 s
        let mut p = Police::new();
        p.tun.v.insert("iRaceTgtAiProbPrcnt".into(), 0.0);
        p.create_units(true);
        p.pick_zone(1, 1);
        p.zone_step(0, &racers, (Vec3::ZERO, 5.0), dt);
        assert_eq!(p.zone.as_ref().map(|z| (z.state, z.target, z.player)), Some((2, Some(10), true)));
        let mut n = 0;
        while !p.zone_go {
            p.zone_step(0, &racers, (FAR, 5.0), dt);
            n += 1;
            assert!(p.zone_on_player() || p.zone_go);
            assert!(n < 1000);
        }
        assert_eq!(n, 166 + 61, "{n}");
        // always the AI (chance 100): the nearest AI racer within fRaceTgtAiDist
        let mut p = Police::new();
        p.tun.v.insert("iRaceTgtAiProbPrcnt".into(), 100.0);
        p.create_units(true);
        p.pick_zone(1, 1);
        p.zone_step(0, &racers, (Vec3::ZERO, 5.0), dt);
        assert_eq!(p.zone.as_ref().map(|z| (z.target, z.player)), Some((Some(12), false)));
        assert!(!p.zone_on_player());
        // its wait runs out: the chase is handed to the update as an AI target (no zone_go for the player)
        for _ in 0..(166 + 61) {
            p.zone_step(0, &racers, (FAR, 5.0), dt);
        }
        assert!(!p.zone_go);
        assert_eq!(p.zone_target, Some(12));
        // the Arrow Trap on an AI racer raises no score (no law record, F-33550); on the player its floor 1000
        p.target = Some(12);
        p.score = 40.0;
        p.start_tactic(TacKind::ArrowTrap, vec![1, 2, 3], true, 25.0);
        assert_eq!(p.score, 40.0);
        p.end_tactic(true);
        assert_eq!((p.target, p.score, p.heat), (None, 40.0, Heat::None));
        p.start_tactic(TacKind::ArrowTrap, vec![1, 2, 3], true, 25.0);
        assert_eq!(p.score, 1000.0);
        // beyond fRaceTgtAiDist: the racer itself
        let mut p = Police::new();
        p.tun.v.insert("iRaceTgtAiProbPrcnt".into(), 100.0);
        p.tun.v.insert("fRaceTgtAiDist".into(), 10.0);
        p.create_units(true);
        p.pick_zone(1, 1);
        p.zone_step(0, &racers, (Vec3::ZERO, 5.0), dt);
        assert_eq!(p.zone.as_ref().map(|z| z.target), Some(Some(10)));
        // the target damaged out: the chase at once
        let wrecked = [(10u32, true, Vec3::ZERO, true)];
        p.zone_step(0, &wrecked, (FAR, 5.0), dt);
        assert!(p.zone_go);
        // BEAT_SH_04B (SetRaceChaseTarget with the player): an AI racer sets it off, the player is the target
        let mut p = Police::new();
        p.tun.v.insert("iRaceTgtAiProbPrcnt".into(), 100.0);
        p.chase_type = PoliceEval::SH_04B.chase_type;
        p.chase_player = PoliceEval::SH_04B.chase_player;
        p.create_units(true);
        p.pick_zone(1, 1);
        p.zone_step(0, &racers, (Vec3::new(40.0, 0.0, 0.0), 5.0), dt);
        assert_eq!(p.zone.as_ref().map(|z| (z.state, z.target, z.player)), Some((2, Some(10), true)));
    }

    #[test]
    fn a_tactic_start_puts_the_phone_away() {
        let mut p = Police::new();
        p.start_tactic(TacKind::Dispatch, vec![], true, 25.0);
        assert!(!p.phone_cut);
        for k in [TacKind::PullOver, TacKind::Pursuit, TacKind::PursuitDbl, TacKind::ArrowTrap] {
            p.phone_cut = false;
            p.start_tactic(k, vec![], true, 25.0);
            assert!(p.phone_cut, "{:?}", k);
        }
    }

    #[test]
    fn the_police_level_for_the_phone() {
        let mut p = Police::new();
        assert_eq!(p.activity(), (0, 0));
        for (h, l) in [(Heat::PullOver, 1), (Heat::Pursuit, 2), (Heat::Search, 2), (Heat::Busted, 2)] {
            p.heat = h;
            assert_eq!(p.activity().0, l);
        }
    }

    #[test]
    fn the_arrest_scene_goes_by_the_cops_of_the_chase() {
        // (CopArrest.sco: RAND_INT_RANGE's upper bound is included - two scenes for one cop, three for more)
        assert_eq!([0, 1].map(|r| arrest_scene(1, r, false, false)), ["cut_cop_arrest_07", "cut_cop_arrest"]);
        assert_eq!([0, 1, 2].map(|r| arrest_scene(3, r, false, false)), ["cut_cop_arrest_02", "cut_cop_arrest_03", "cut_cop_arrest_08"]);
        // (the wiped-out biker's scene before the count and the payback; the payback's walk-ups 1 / 2 each)
        assert_eq!(arrest_scene(3, 0, true, true), "cut_cop_arrest_bikestand");
        assert_eq!([0, 1].map(|r| arrest_scene(1, r, false, true)), ["payback_goons_walkup_02", "payback_goons_walkup_01"]);
        assert_eq!(arrest_gesture("payback_goons_walkup_02"), Some((crate::drivers::Gesture::ListenLeft, 0.3333, 9.0)));
        assert_eq!(arrest_gesture("cut_cop_arrest_07"), Some((crate::drivers::Gesture::CopHandsUp, 3.5, 1.5)));
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
