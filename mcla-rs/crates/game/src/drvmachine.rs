//! The in-car driver's animation state machine, mcCarAnimMachine (vtable slots: F-74243 symbols, F-65927 graph; the bike's
//! mcMotorcycleAnimMachine is `drivers.rs::ride_pose`). The machine reads its context (mcAnimMachine +28, filled every frame
//! by the vehicle gatherer F-28290 and the context step) and the shared tune (context +16, mcAnimMachineConstantData).
//!
//! Its step (F-58452): the current state's update, then up to three times the first transition of the state whose test
//! is true (F-15065: a condition function, or an RPN string through the expression evaluator F-39484): the state's exit, the
//! transition's action, the target's enter and update. The graph below is the builder's, in its order (checked against the
//! builder run offline: `work/drvmachine/table.json`).

/// The machine's context (mcAnimMachine +28): the fields the car machine reads.
#[derive(Clone, Copy, Debug, Default)]
pub struct Ctx {
    /// +4: the gesture (20 = none, 19 = find_sh_04)
    pub gesture: i32,
    /// +8: the event flags (`F_*`)
    pub flags: u32,
    /// +12: the machine's own flags (bit 2 / 3: steering locked)
    pub flags2: u32,
    /// +20 ReadGoalSteering, +24 ReadSteering, +28 ReadSmoothedSteeringParam
    pub goal: f32,
    pub steer: f32,
    pub smooth: f32,
    /// +36 ReadSmoothedSteeringDeltaToLimit, +40 ReadDSteeringDt
    pub to_limit: f32,
    pub dsteer: f32,
    /// +48 IsUpdateCulled, +49 IsOpponentDefined
    pub culled: bool,
    pub opponent: bool,
    /// +64: the steering wheel's angle scale (ReadSteeringWheelAngle = +64 x +28)
    pub wheel_scale: f32,
    /// +73 IsSteeringToLeft, +74 IsSteeringToRight
    pub to_left: bool,
    pub to_right: bool,
    /// +76 ReadSkidTimer, +80 ReadSteeringNeutralTimer, +84 ReadSteeringMaxedTimer
    pub skid_timer: f32,
    pub neutral_timer: f32,
    pub maxed_timer: f32,
    /// +96 ReadSpeed, +108 ReadAcceleration, +112 ReadLateralAcceleration
    pub speed: f32,
    pub accel: f32,
    pub lat_accel: f32,
    /// +124: the shift (+1 up, -1 down)
    pub shift: i32,
    /// +128 ReadSkidValue
    pub skid: f32,
    /// +148 / +152 / +156 ReadTimerA / B / C, +160 ReadRandomNum, +164 ReadDt
    pub timer_a: f32,
    pub timer_b: f32,
    pub timer_c: f32,
    pub random: f32,
    pub dt: f32,
}

/// Context +8 bits (the readers' masks).
pub const F_IMPACT_LEFT: u32 = 1 << 0;
pub const F_IMPACT_RIGHT: u32 = 1 << 1;
pub const F_IMPACT_FRONT_SOFT: u32 = 1 << 2;
pub const F_IMPACT_FRONT_HARD: u32 = 1 << 3;
pub const F_NITRO: u32 = 1 << 4;
pub const F_REVERSING: u32 = 1 << 7;
pub const F_VISIBLE_DAMAGE: u32 = 1 << 9;
pub const F_COCKPIT_IDLE_TIMEUP: u32 = 1 << 10;
pub const F_UPSIDE_DOWN: u32 = 1 << 11;
pub const F_IMPACT_DRIVEABLE: u32 = 1 << 14;
pub const F_IMPACT_DAMAGEOUT: u32 = 1 << 16;
pub const F_DAMAGEOUT_TIMING: u32 = 1 << 18;
pub const F_ONLINE: u32 = 1 << 24;
pub const F_RACE_FINISH: u32 = 1 << 25;
pub const F_GPS_WARPING: u32 = 1 << 26;
pub const F_HONKING: u32 = 1 << 27;
pub const F_POST_PAUSE_MENU: u32 = 1 << 29;

/// mcAnimMachineConstantData (context +16): the fields the car machine reads.
#[derive(Clone, Copy, Debug, Default)]
pub struct Tune {
    /// +4 IsEnabledCruiseSteering, +5 ..HandOverHandSteeringInCockpit, +6 ..PalmSteering, +7 ..Shifting, +8 ..Fidgets, +9 ..Impacts
    pub enabled: [bool; 6],
    /// +48 / +52 / +56 / +60: the stopped / low / medium / high speed rolling bands (mph; x 0.44704 to m/s)
    pub bands: [f32; 4],
    /// +72: the steering scale the start's action F-34905 hands to F-34842
    pub start_steer: f32,
    /// +92 HandOverHandSkidTimeWindow
    pub hoh_skid_window: f32,
    /// +136 / +140 / +144 Impact Hard / Soft / Side ThresholdAccel
    pub impact: [f32; 3],
}

/// What the machine reads of its driver (the driver's anim player) and of the game.
#[derive(Clone, Copy, Debug, Default)]
pub struct Env {
    /// ReadTimeRemaining (F-87745): the playing clip's time left
    pub remaining: f32,
    /// ReadAnimPhase (F-96834): the playing clip's phase
    pub phase: f32,
    /// IsCutscenePlaying (F-91845)
    pub cutscene: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    StartHere,
    StoppedIdle,
    StoppedIdleTimeup,
    Cruising,
    Suspended,
    WindHohLeft,
    UnwindHohLeft,
    SlipHohLeft,
    WindHohRight,
    UnwindHohRight,
    SlipHohRight,
    Shifting,
    IntoReverse,
    Reversing,
    OutofReverse,
    NosBoost,
    HonkTheHorn,
    ImpactSoft,
    ImpactDriveable,
    ImpactDamageout,
    ImpactDamageoutInverted,
    ImpactDamageoutInvertedCarDone,
    ImpactStartCar,
    ImpactStartCarDone,
    Fidget,
    GestureSwitch,
}

/// The builder's states, in its order, with their names.
pub const STATES: [(State, &str); 26] = [
    (State::StartHere, "start here"),
    (State::StoppedIdle, "stopped idle"),
    (State::StoppedIdleTimeup, "stopped idle timeup"),
    (State::Cruising, "cruising"),
    (State::Suspended, "suspended"),
    (State::WindHohLeft, "wind into hand over hand left"),
    (State::UnwindHohLeft, "unwind outof hand over hand left"),
    (State::SlipHohLeft, "slip outof hand over hand left"),
    (State::WindHohRight, "wind into hand over hand right"),
    (State::UnwindHohRight, "unwind outof hand over hand right"),
    (State::SlipHohRight, "slip outof hand over hand right"),
    (State::Shifting, "shifting"),
    (State::IntoReverse, "into reverse"),
    (State::Reversing, "reversing"),
    (State::OutofReverse, "outof reverse"),
    (State::NosBoost, "nos boost"),
    (State::HonkTheHorn, "honk the horn"),
    (State::ImpactSoft, "impact soft"),
    (State::ImpactDriveable, "impact driveable"),
    (State::ImpactDamageout, "impact damageout"),
    (State::ImpactDamageoutInverted, "impact damageout inverted"),
    (State::ImpactDamageoutInvertedCarDone, "impact damageout inverted car done"),
    (State::ImpactStartCar, "impact start car"),
    (State::ImpactStartCarDone, "impact start car done"),
    (State::Fidget, "fidget"),
    (State::GestureSwitch, "gesture switch"),
];

/// A transition's test: a condition function of the builder (named by its address in the doc) or an RPN string.
#[derive(Clone, Copy, Debug)]
pub enum Test {
    Always,
    Culled,
    Gesture,
    NitroOn,
    NitroOff,
    CockpitIdleTimeup,
    Moving,
    FidgetTime,
    Damageout,
    DamageoutInverted,
    Driveable,
    Forward,
    IntoReverse,
    Stopped,
    ImpactSoft,
    WindLeft,
    WindRight,
    Shift,
    SlipLeft,
    UnwindLeft,
    UnwoundLeft,
    ClipEnding,
    SlipRight,
    UnwindRight,
    UnwoundRight,
    OutofReverse,
    NosDamageout,
    NosDamageoutInverted,
    NosDriveable,
    StartCarDone,
    CarDone,
    Rpn(&'static str),
}

const HONK: &str = "$IsHonking $ReadGoalSteering $Abs %0.015 < & $ReadSmoothedSteeringParam $Abs %0.1 < &";
const RACE_FINISH: &str = "$IsRaceFinish";

/// Each state's transitions in the builder's order (first true wins).
pub fn transitions(s: State) -> &'static [(Test, State)] {
    use State::*;
    use Test::*;
    match s {
        StartHere => &[(Always, StoppedIdle)],
        StoppedIdle => &[
            (Gesture, GestureSwitch),
            (CockpitIdleTimeup, StoppedIdleTimeup),
            (Moving, Cruising),
            (FidgetTime, Fidget),
            (Damageout, ImpactDamageout),
            (DamageoutInverted, ImpactDamageoutInverted),
            (Driveable, ImpactDriveable),
            (Rpn(HONK), HonkTheHorn),
        ],
        StoppedIdleTimeup => &[
            (Forward, Cruising),
            (Test::IntoReverse, State::IntoReverse),
            (Damageout, ImpactDamageout),
            (DamageoutInverted, ImpactDamageoutInverted),
            (Driveable, ImpactDriveable),
            (Rpn(HONK), HonkTheHorn),
        ],
        Cruising => &[
            (Culled, Suspended),
            (Gesture, GestureSwitch),
            (CockpitIdleTimeup, StoppedIdleTimeup),
            (Stopped, StoppedIdle),
            (
                Rpn("$IsImpactDamageout $IsUpsideDown ! & $IsOpponentDefined ! & $IsEnabledImpacts & $IsDamageoutTiming ! & $IsOnline ! &"),
                ImpactDamageout,
            ),
            (DamageoutInverted, ImpactDamageoutInverted),
            (Driveable, ImpactDriveable),
            (Test::ImpactSoft, State::ImpactSoft),
            (NitroOn, NosBoost),
            (WindLeft, WindHohLeft),
            (WindRight, WindHohRight),
            (Shift, Shifting),
            (Test::IntoReverse, State::IntoReverse),
        ],
        Suspended => &[
            (Damageout, ImpactDamageout),
            (DamageoutInverted, ImpactDamageoutInverted),
            (Driveable, ImpactDriveable),
            (Rpn("$IsUpdateCulled !"), Cruising),
        ],
        WindHohLeft => &[(SlipLeft, SlipHohLeft), (UnwindLeft, UnwindHohLeft)],
        UnwindHohLeft => &[(UnwoundLeft, Cruising)],
        SlipHohLeft => &[(ClipEnding, Cruising)],
        WindHohRight => &[(SlipRight, SlipHohRight), (UnwindRight, UnwindHohRight)],
        UnwindHohRight => &[(UnwoundRight, Cruising)],
        SlipHohRight => &[(ClipEnding, Cruising)],
        Shifting => &[(ClipEnding, Cruising)],
        State::IntoReverse => &[(ClipEnding, Reversing)],
        Reversing => &[(Test::OutofReverse, State::OutofReverse)],
        State::OutofReverse => &[(ClipEnding, Cruising)],
        NosBoost => &[
            (NosDamageout, ImpactDamageout),
            (NosDamageoutInverted, ImpactDamageoutInverted),
            (NosDriveable, ImpactDriveable),
            (NitroOff, Cruising),
        ],
        HonkTheHorn => &[(
            Rpn("$IsHonking ! $ReadTimeRemaining %0.033 < & $ReadGoalSteering $Abs %0.015 > | $ReadSpeed $Abs %1.0 > |"),
            StoppedIdleTimeup,
        )],
        State::ImpactSoft => &[(ClipEnding, Cruising)],
        ImpactDriveable => &[(ClipEnding, Cruising)],
        ImpactDamageout => &[(Rpn(RACE_FINISH), Cruising), (ClipEnding, ImpactStartCar)],
        ImpactDamageoutInverted => &[
            (Rpn(RACE_FINISH), Cruising),
            (Rpn("$ReadTimeRemaining %0.033 < $IsGPSWarping ! &"), ImpactDamageoutInvertedCarDone),
        ],
        ImpactDamageoutInvertedCarDone => &[(CarDone, Cruising)],
        ImpactStartCar => &[(Rpn(RACE_FINISH), Cruising), (StartCarDone, ImpactStartCarDone)],
        ImpactStartCarDone => &[(CarDone, Cruising)],
        Fidget => &[
            (Gesture, GestureSwitch),
            (Rpn("$ReadTimeRemaining %0.033 < $ReadSmoothedSteeringParam $Abs %0.1 > | $IsCutscenePlaying |"), StoppedIdle),
        ],
        GestureSwitch => &[(
            Rpn("$ReadTimeRemaining %0.033 < $IsGestureNotPlaying | $IsPostPauseMenu | $IsGestureFindSH04 $IsCutscenePlaying ! & |"),
            Cruising,
        )],
    }
}

/// `!(a <= b)`, `!(a >= b)`: the original's branches on fcmpu (an unordered compare takes neither branch).
fn not_le(a: f32, b: f32) -> bool {
    !(a <= b)
}
fn not_ge(a: f32, b: f32) -> bool {
    !(a >= b)
}

/// A transition's test (the builder's condition functions; the RPN strings through `rpn`).
pub fn test(t: Test, c: &Ctx, k: &Tune, e: &Env) -> bool {
    let f = |b: u32| c.flags & b != 0;
    let impacts = k.enabled[5];
    // the damage-out family (F-12294 / F-61592 / F-80138; the nitro state's F-14305 / F-66048 / F-35021 skip the opponent test)
    let damageout = |inverted: bool, opp: bool| {
        f(F_IMPACT_DAMAGEOUT) && f(F_UPSIDE_DOWN) == inverted && (opp || !c.opponent) && impacts && !f(F_DAMAGEOUT_TIMING)
    };
    let driveable = |opp: bool| f(F_IMPACT_DRIVEABLE) && (opp || !c.opponent) && impacts && !f(F_DAMAGEOUT_TIMING);
    // the hand-over-hand winds (F-78706 / F-64935): rolling, skidding within the window or timer B past 1, timer A past 0.25,
    // near the limit, the smoothed steering past 0.9, a human, the cockpit's hand-over-hand on
    let wind = |past: bool| {
        not_le(c.speed, 1.0)
            && (c.skid_timer < k.hoh_skid_window || not_le(c.timer_b, 1.0))
            && not_le(c.timer_a, 0.25)
            && not_ge(c.to_limit, 0.15)
            && past
            && !c.opponent
            && k.enabled[1]
    };
    match t {
        Test::Always => true,
        Test::Culled => c.culled,
        Test::Gesture => c.gesture != 20,
        Test::NitroOn => f(F_NITRO) && c.timer_a > 0.25 && !c.opponent,
        Test::NitroOff => !f(F_NITRO),
        Test::CockpitIdleTimeup => f(F_COCKPIT_IDLE_TIMEUP),
        Test::Moving => c.goal.abs() > 0.015 || not_le(c.speed.abs(), 1.0),
        Test::FidgetTime => not_le(c.timer_a, 15.0) && not_ge(c.smooth.abs(), 0.1) && !e.cutscene && !c.opponent,
        Test::Damageout => damageout(false, false),
        Test::DamageoutInverted => damageout(true, false),
        Test::Driveable => driveable(false),
        Test::Forward => c.speed > 1.0,
        Test::IntoReverse => {
            not_ge(c.speed, -1.0) && f(F_REVERSING) && not_le(c.timer_a, 0.25) && not_ge(c.smooth.abs(), 0.1) && !c.opponent
        }
        Test::Stopped => {
            not_ge(c.goal.abs(), 0.015) && not_ge(c.smooth.abs(), 0.015) && not_ge(c.speed.abs(), 1.0) && c.timer_a > 0.25
        }
        Test::ImpactSoft => f(F_VISIBLE_DAMAGE) && !c.opponent && impacts,
        Test::WindLeft => wind(not_ge(c.smooth, -0.9)),
        Test::WindRight => wind(not_le(c.smooth, 0.9)),
        Test::Shift => {
            (c.shift == 1 || c.shift == -1) && !(c.speed < 1.0) && not_le(c.timer_a, 0.25) && k.enabled[3] && !c.opponent
        }
        Test::SlipLeft => not_le(c.dsteer, 2.5) && c.to_limit < 0.1,
        Test::UnwindLeft => c.goal > -0.9 || not_ge(c.speed, 1.0),
        Test::UnwoundLeft => c.smooth > -0.25,
        Test::ClipEnding => e.remaining < 0.033,
        Test::SlipRight => not_ge(c.dsteer, -2.5) && c.to_limit < 0.1,
        Test::UnwindRight => c.goal < 0.9 || not_ge(c.speed, 1.0),
        Test::UnwoundRight => c.smooth < 0.25,
        Test::OutofReverse => {
            let settled = !(c.timer_a < 0.25) && !f(F_REVERSING) && c.smooth.abs() < 0.1;
            settled || f(F_IMPACT_DAMAGEOUT) || f(F_IMPACT_DRIVEABLE)
        }
        Test::NosDamageout => damageout(false, true),
        Test::NosDamageoutInverted => damageout(true, true),
        Test::NosDriveable => driveable(true),
        Test::StartCarDone => c.timer_c > 4.0,
        Test::CarDone => f(F_POST_PAUSE_MENU) || not_le(c.timer_c, 5.0),
        Test::Rpn(s) => rpn(s, c, k, e),
    }
}

/// A symbol of the expression table (registered by F-74243): its value, booleans as 1 / 0.
fn symbol(name: &str, c: &Ctx, k: &Tune, e: &Env) -> Option<f32> {
    let f = |b: u32| c.flags & b != 0;
    let b = |x: bool| if x { 1.0 } else { 0.0 };
    let band = |i: usize| k.bands[i] * 0.44704;
    Some(match name {
        "IsEnabledCruiseSteering" => b(k.enabled[0]),
        "IsEnabledHandOverHandSteeringInCockpit" => b(k.enabled[1]),
        "IsEnabledPalmSteering" => b(k.enabled[2]),
        "IsEnabledShifting" => b(k.enabled[3]),
        "IsEnabledFidgets" => b(k.enabled[4]),
        "IsEnabledImpacts" => b(k.enabled[5]),
        "IsOpponentDefined" => b(c.opponent),
        "StoppedRolling" => b(c.speed <= band(0)),
        // (Low / Medium / High / ExtremeSpeedRolling: not in the car graph, not ported)
        "ImpactHardThresholdAccel" => k.impact[0],
        "ImpactSoftThresholdAccel" => k.impact[1],
        "ImpactSideThresholdAccel" => k.impact[2],
        "IsImpactFromLeft" => b(f(F_IMPACT_LEFT)),
        "IsImpactFromRight" => b(f(F_IMPACT_RIGHT)),
        "IsImpactFromFrontSoft" => b(f(F_IMPACT_FRONT_SOFT)),
        "IsImpactFromFrontHard" => b(f(F_IMPACT_FRONT_HARD)),
        "IsSteeringLocked" => b(c.flags2 & 0xC != 0),
        "IsSteeringToLeft" => b(c.to_left),
        "IsSteeringToRight" => b(c.to_right),
        "IsUpShifting" => b(c.shift == 1),
        "IsDownShifting" => b(c.shift == -1),
        "IsUpsideDown" => b(f(F_UPSIDE_DOWN)),
        "IsReversing" => b(f(F_REVERSING)),
        "IsCockpitIdleTimeup" => b(f(F_COCKPIT_IDLE_TIMEUP)),
        "IsNitroBurn" => b(f(F_NITRO)),
        "IsHonking" => b(f(F_HONKING)),
        "IsGPSWarping" => b(f(F_GPS_WARPING)),
        "IsImpactDriveable" => b(f(F_IMPACT_DRIVEABLE)),
        "IsImpactDamageout" => b(f(F_IMPACT_DAMAGEOUT)),
        "IsDamageoutTiming" => b(f(F_DAMAGEOUT_TIMING)),
        "IsPostPauseMenu" => b(f(F_POST_PAUSE_MENU)),
        "IsVisibleDamage" => b(f(F_VISIBLE_DAMAGE)),
        "IsOnline" => b(f(F_ONLINE)),
        "IsRaceFinish" => b(f(F_RACE_FINISH)),
        "IsCutscenePlaying" => b(e.cutscene),
        "IsUpdateCulled" => b(c.culled),
        "IsGestureNeutral" => b(c.gesture == 0),
        "IsGestureLeft" => b(c.gesture == 1),
        "IsGestureRight" => b(c.gesture == 2),
        "IsGestureTalkLeft" => b(c.gesture == 3),
        "IsGestureTalkRight" => b(c.gesture == 4),
        "IsGestureMild" => b(c.gesture == 5),
        "IsGestureWin" => b(c.gesture == 6),
        "IsGestureListenLeft" => b(c.gesture == 7),
        "IsGestureListenRight" => b(c.gesture == 8),
        "IsGestureThumbDownLeft" => b(c.gesture == 9),
        "IsGestureThumbDownRight" => b(c.gesture == 10),
        "IsGestureCasualRight" => b(c.gesture == 11),
        "IsGestureMildRight" => b(c.gesture == 13),
        "IsGestureCopFidget" => b(c.gesture == 17),
        "IsGestureCopHandsUp" => b(c.gesture == 18),
        "IsGestureFindSH04" => b(c.gesture == 19),
        "IsGestureNotPlaying" => b(c.gesture == 20),
        "ReadSpeed" => c.speed,
        "ReadGoalSteering" => c.goal,
        "ReadSteering" => c.steer,
        "ReadSmoothedSteeringDeltaToLimit" => c.to_limit,
        "ReadAcceleration" => c.accel,
        "ReadLateralAcceleration" => c.lat_accel,
        "ReadDSteeringDt" => c.dsteer,
        "ReadSkidValue" => c.skid,
        "ReadTimerA" => c.timer_a,
        "ReadTimerB" => c.timer_b,
        "ReadTimerC" => c.timer_c,
        "HandOverHandSkidTimeWindow" => k.hoh_skid_window,
        "ReadSkidTimer" => c.skid_timer,
        "ReadSteeringNeutralTimer" => c.neutral_timer,
        "ReadSteeringMaxedTimer" => c.maxed_timer,
        "ReadRandomNum" => c.random,
        "ReadDt" => c.dt,
        "StoppedRollingBandCutoff" => band(0),
        "LowSpeedRollingBandCutoff" => band(1),
        "MedSpeedRollingBandCutoff" => band(2),
        "HighSpeedRollingBandCutoff" => band(3),
        "ReadTimeRemaining" => e.remaining,
        "ReadAnimPhase" => e.phase,
        "ReadSteeringWheelAngle" => c.wheel_scale * c.smooth,
        "ReadSmoothedSteeringParam" => c.smooth,
        _ => return None,
    })
}

/// An RPN condition (the expression table's evaluator): `$Name` a symbol (`$Abs`, `$Min`, `$Max` the functions), `%x` a number,
/// `<` `>` compare, `&` `|` `!` logic; true when the result is not 0.
pub fn rpn(s: &str, c: &Ctx, k: &Tune, e: &Env) -> bool {
    let mut st: Vec<f32> = Vec::new();
    let b = |x: bool| if x { 1.0 } else { 0.0 };
    for tok in s.split_whitespace() {
        let v = if let Some(n) = tok.strip_prefix('$') {
            match n {
                "Abs" => st.pop().unwrap_or(0.0).abs(),
                "Min" | "Max" => {
                    let y = st.pop().unwrap_or(0.0);
                    let x = st.pop().unwrap_or(0.0);
                    if n == "Min" { if x - y >= 0.0 { y } else { x } } else if x - y >= 0.0 { x } else { y }
                }
                _ => symbol(n, c, k, e).unwrap_or(0.0),
            }
        } else if let Some(x) = tok.strip_prefix('%') {
            x.parse().unwrap_or(0.0)
        } else {
            let unary = tok == "!";
            let y = st.pop().unwrap_or(0.0);
            if unary {
                b(y == 0.0)
            } else {
                let x = st.pop().unwrap_or(0.0);
                match tok {
                    "<" => b(x < y),
                    ">" => b(x > y),
                    "&" => b(x != 0.0 && y != 0.0),
                    "|" => b(x != 0.0 || y != 0.0),
                    _ => 0.0,
                }
            }
        };
        st.push(v);
    }
    st.pop().map_or(false, |v| v != 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every transition's test against the original's (the builder's graph, its test F-15065 and the expression evaluator, run
    /// offline on 8,000 random contexts: `work/drvmachine/cond_ref.py`).
    #[test]
    fn transition_tests_match_the_original() {
        let mut n = 0;
        for line in include_str!("testdata/drvmachine_cond_ref.txt").lines() {
            let (a, want) = line.split_once(" -> ").unwrap();
            let w: Vec<u32> = a.split_whitespace().map(|x| x.parse().unwrap()).collect();
            let f = |i: usize| f32::from_bits(w[i]);
            let c = Ctx {
                gesture: w[0] as i32,
                flags: w[1],
                flags2: w[2],
                goal: f(3),
                steer: f(4),
                smooth: f(5),
                to_limit: f(6),
                dsteer: f(7),
                culled: w[8] >> 24 != 0,
                opponent: (w[8] >> 16) & 0xff != 0,
                wheel_scale: f(9),
                to_left: (w[10] >> 16) & 0xff != 0,
                to_right: (w[10] >> 8) & 0xff != 0,
                skid_timer: f(11),
                neutral_timer: f(12),
                maxed_timer: f(13),
                speed: f(14),
                accel: f(15),
                lat_accel: f(16),
                shift: w[17] as i32,
                skid: f(18),
                timer_a: f(19),
                timer_b: f(20),
                timer_c: f(21),
                random: f(22),
                dt: f(23),
            };
            let by = |x: u32, i: u32| (x >> (24 - 8 * i)) & 0xff != 0;
            let k = Tune {
                enabled: [by(w[24], 0), by(w[24], 1), by(w[24], 2), by(w[24], 3), by(w[25], 0), by(w[25], 1)],
                bands: [f(26), f(27), f(28), f(29)],
                start_steer: f(30),
                hoh_skid_window: f(31),
                impact: [f(32), f(33), f(34)],
            };
            let e = Env { remaining: f(35), phase: f(36), cutscene: w[37] != 0 };
            let mut got = String::new();
            for (s, _) in STATES {
                for (t, _) in transitions(s) {
                    got.push(if test(*t, &c, &k, &e) { '1' } else { '0' });
                }
            }
            assert_eq!(got, want, "{line}");
            n += 1;
        }
        assert_eq!(n, 7981);
    }
}
