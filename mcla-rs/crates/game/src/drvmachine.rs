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
    /// +32: added to the smoothed steering for the sweeps' phase (F-65297, F-43516, F-99138)
    pub steer_add: f32,
    /// +36 ReadSmoothedSteeringDeltaToLimit, +40 ReadDSteeringDt
    pub to_limit: f32,
    pub dsteer: f32,
    /// +48 IsUpdateCulled, +49 IsOpponentDefined
    pub culled: bool,
    pub opponent: bool,
    /// +56 / +60: the steering smoothing's parameters (the states write them, the context step reads them)
    pub smooth_k: [f32; 2],
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
    /// +64 / +68: the smoothing parameters while cruising (to context +56 / +60)
    pub cruise_k: [f32; 2],
    /// +72: the steering scale the start's action F-34905 hands to F-34842 (and the hand-over-hand and reversing exits)
    pub start_steer: f32,
    /// +76: reversing's steering scale, +84: hand over hand's
    pub reverse_steer: f32,
    pub hoh_steer: f32,
    /// +92 HandOverHandSkidTimeWindow
    pub hoh_skid_window: f32,
    /// +96: the steering where the hand-over-hand clip's phase starts
    pub hoh_start: f32,
    /// +100 / +104 wind, +108 / +112 unwind, +116 / +120 slip: the smoothing parameters of the hand-over-hand states
    pub wind_k: [f32; 2],
    pub unwind_k: [f32; 2],
    pub slip_k: [f32; 2],
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

// ---------------------------------------------------------------------------------------------- the step and its callbacks

impl Tune {
    /// mcAnimMachineConstantData as its constructor F-40144 fills it.
    pub fn original() -> Tune {
        Tune {
            enabled: [true, true, true, true, false, true],
            bands: [0.15, 25.0, 55.0, 100.0],
            cruise_k: [7.0, 20.0],
            start_steer: 1.8,
            reverse_steer: 1.5,
            hoh_steer: std::f32::consts::TAU,
            hoh_skid_window: 1.5,
            hoh_start: 0.27,
            wind_k: [6.0, 20.0],
            unwind_k: [4.5, 15.0],
            slip_k: [10.0, 20.0],
            impact: [100.0, 25.0, 25.0],
        }
    }
}

/// A clip of the driver's dictionary by name (the lookup F-66148 is case-blind).
pub type Clip = &'static str;

/// What the callbacks ask of the driver's anim player and of the game, in call order.
#[derive(Clone, Debug, PartialEq)]
pub enum Cmd {
    /// F-24441: play the clip, blending over the time; its two flags
    Play(Clip, f32, bool, bool),
    /// F-73665: the find_sh_04 gesture's play
    PlayCut(Clip, f32, bool, bool),
    /// F-85393: a pair of clips (A, B), its flag, the blend time
    Pair(Clip, Clip, bool, f32),
    /// F-45908: five clips (the 90 / 45 / centre / 45 / 90 sweep), the blend time, its flag
    Five([Clip; 5], f32, bool),
    /// F-40742 the clip's phase, F-53423 its rate, F-84631 the five's rate, F-62772 the pair's phase, F-94065 the five's
    /// phase, F-30180 every layer's phase, F-22341 the pair's weight
    Phase(f32),
    Rate(f32),
    Rate5(f32),
    Phase2(f32),
    Phase5(f32),
    PhaseAll(f32),
    Weight(f32),
    /// F-19950: an overlay clip, the blend time, its flag
    Overlay(Clip, f32, bool),
    /// the racer's vehicle +16: vt+56(1) when the start-car impact begins, vt+52(0) when it is done
    EngineStart,
    EngineStop,
    /// F-49831: the camera manager takes the racer back (the inverted and start-car impacts' exits)
    HandBack,
}

/// What the machine reads of its racer each frame (machine +40).
#[derive(Clone, Copy, Debug, Default)]
pub struct Racer {
    /// vt+40: the local human (the faces, the pan overlay and the neutral gesture's skip ask it)
    pub human: bool,
    /// +656 == 3 (with the human: the neutral gesture skips `_rspnd_rgt`, else `_rspnd_lft`)
    pub mode3: bool,
    /// the vehicle's sim (+48 +8) vt+52: alarmed above 0.5 while cruising
    pub alarm: f32,
    /// the sim's +592 bit 5: the start-car impact does not start the engine
    pub engine_off: bool,
}

const PAN: Clip = "DRV_PAN_SWEEP_OVERLAY";
const IDLE_STP: Clip = "DRV_IDLE_STP";
const STEER_SWEEP: Clip = "DRV_STEER_SWEEP";
const REV: Clip = "DRV_REV";
const FIND_SH: Clip = "drv_find_sh_04";
const FIND_SH_MOVER: Clip = "mover_player_drv00_car_name_drv_mp_01_set";
/// The gesture pickers' tables (the round robins F-80898 .. F-57037; counters at machine +68 .. +100).
const NEUTRAL: [Clip; 5] = ["drv_gest_mirror", "drv_win", "drv_gest_rspnd_lft", "drv_gest_rspnd_rgt", "drv_gest_fidget_01"];
const LEFT: [Clip; 5] = ["drv_gest_mild_lft_01", "drv_gest_agro_lft_01", "drv_gest_thm_dwn_lft", "drv_gest_thm_up_lft", "drv_gest_cas_lft_01"];
const RIGHT: [Clip; 5] = ["drv_gest_mild_rgt_01", "drv_gest_agro_rgt_01", "drv_gest_thm_dwn_rgt", "drv_gest_thm_up_rgt", "drv_gest_cas_rgt_01"];
const TALK_LEFT: [Clip; 4] = ["drv_gest_cas_lft_01", "drv_gest_agro_lft_01", "drv_gest_mild_lft_01", "drv_gest_rspnd_lft"];
const TALK_RIGHT: [Clip; 4] = ["drv_gest_cas_rgt_01", "drv_gest_agro_rgt_01", "drv_gest_mild_rgt_01", "drv_gest_rspnd_rgt"];
const MILD: [Clip; 2] = ["drv_gest_mild_lft_01", "drv_gest_mild_rgt_01"];
const WIN: [Clip; 2] = ["drv_win", "drv_win_02"];
const FIDGET: [Clip; 2] = ["drv_gest_fidget_01", "drv_gest_mirror"];

/// `fsel(a, b, c)`: b when a >= 0 (a NaN takes c).
fn fsel(a: f32, b: f32, c: f32) -> f32 {
    if a >= 0.0 {
        b
    } else {
        c
    }
}

/// 70 mph and 80 mph in m/s, as the executable holds them (70 mph is one unit under the nearest single).
const MPH70: f32 = f32::from_bits(0x41FA_57A7);
const MPH80: f32 = f32::from_bits(0x420F_0D84);

/// clamp((speed - 70 mph) x (1 / 40 mph), 0, 1) as the cruising and shifting updates compute it.
fn fast(speed: f32) -> f32 {
    let x = (speed - MPH70) * f32::from_bits(0x3D65_0FF0);
    let a = fsel(x, x, 0.0);
    fsel(a - 1.0, 1.0, a)
}

/// The machine's own state (machine +20, +44, +45, +68 .. +108) and what it keeps of its driver (+104 face, +112 overlay).
#[derive(Clone, Debug)]
pub struct Machine {
    pub state: State,
    /// +108: the cruising face state (0 none yet, 1 neutral, 2 anger, 3 alarmed)
    pub face_state: i32,
    /// +45: the gestures add the jaw flap overlay (cleared by the start's action)
    pub jawflap: bool,
    /// +68 .. +100: the pickers' counters (neutral, left, right, talk left, talk right, mild, win, fidget, disappointed)
    pub counters: [i32; 9],
    /// the racer's character set (+144): the faces are `<name>_<set>`
    pub set: String,
    /// driver +104: the face clip (None = 0)
    pub face: Option<String>,
    /// driver +112: the overlay clip F-63058 sets (None = 0)
    pub overlay: Option<Clip>,
    /// the racer's +684, set to 20 by the gesture exit (F-90946)
    pub racer_684: i32,
    /// the shared multiply-with-carry generator F-84930 draws from (a global of the game)
    pub rng: [u32; 2],
    /// the game manager's +300 (a global): the find_sh_04 gesture is playing
    pub find_sh: bool,
}

impl Machine {
    pub fn new(set: &str, rng: [u32; 2]) -> Machine {
        Machine {
            state: State::StartHere,
            face_state: 0,
            jawflap: false,
            counters: [0; 9],
            set: set.to_string(),
            face: None,
            overlay: None,
            racer_684: 0,
            rng,
            find_sh: false,
        }
    }

    /// F-58452: the current state's update, then up to three transitions (exit, action, enter, update).
    pub fn step(&mut self, c: &mut Ctx, k: &Tune, e: &Env, r: &Racer, out: &mut Vec<Cmd>) {
        self.update(c, k, e, r, out);
        for _ in 0..3 {
            let Some(&(_, to)) = transitions(self.state).iter().find(|(t, _)| test(*t, c, k, e)) else { break };
            self.exit(c, k, r, out);
            if self.state == State::StartHere {
                self.start(c, k, r, out);
            }
            self.state = to;
            self.enter(c, k, r, out);
            self.update(c, k, e, r, out);
        }
    }

    /// F-52181: a face for the human (`<name>_<set>`), else 0.
    fn face_of(&self, name: &str, r: &Racer) -> Option<String> {
        r.human.then(|| format!("{name}_{}", self.set))
    }

    /// F-84930: context +160 = lo + (hi - lo) x u, u the generator's low 23 bits x 2^-23.
    fn random(&mut self, c: &mut Ctx, lo: f32, hi: f32) {
        let t = self.rng[0] as u64 * 0x5CDC_FAA7 + self.rng[1] as u64;
        self.rng = [t as u32, (t >> 32) as u32];
        let v = (t & 0x7F_FFFF) as f32;
        c.random = ((hi - lo) * v).mul_add(1.192_092_9e-7, lo);
    }

    /// F-34842: a new steering wheel scale, the steering rescaled so the wheel's angle (+64 x +28) stays.
    fn rescale(c: &mut Ctx, s: f32) {
        let q = c.wheel_scale / s;
        c.wheel_scale = s;
        c.steer *= q;
        c.smooth *= q;
    }

    /// F-56423 with F-63058: the pan overlay for the human, 0 for the others.
    fn pan(&mut self, r: &Racer) {
        self.overlay = r.human.then_some(PAN);
    }

    /// The start's action F-34905.
    fn start(&mut self, c: &mut Ctx, k: &Tune, r: &Racer, out: &mut Vec<Cmd>) {
        Self::rescale(c, k.start_steer);
        c.timer_c = 0.0;
        self.pan(r);
        out.push(Cmd::Play(IDLE_STP, 0.0, true, true));
        out.push(Cmd::PhaseAll(0.0));
        self.jawflap = false;
    }

    fn update(&mut self, c: &mut Ctx, k: &Tune, e: &Env, r: &Racer, out: &mut Vec<Cmd>) {
        use State::*;
        let half = |c: &Ctx| ((c.steer_add + c.smooth) + 1.0) * 0.5;
        match self.state {
            Cruising => self.cruise(c, k, r, out),
            // F-43516
            Shifting => {
                out.push(Cmd::Phase5(half(c)));
                out.push(Cmd::Rate5(fast(c.speed).mul_add(5.0, 1.0)));
            }
            // F-79319
            Reversing => {
                out.push(Cmd::Phase((c.smooth + 1.0) * 0.5));
                if c.flags & F_REVERSING == 0 {
                    c.flags2 |= 8;
                }
            }
            // F-27834 (left: the steering's part below 0), F-38635 (right: above 0), from tune +96 to full lock
            WindHohLeft | UnwindHohLeft | WindHohRight | UnwindHohRight => {
                let s = c.smooth;
                let side = if matches!(self.state, WindHohLeft | UnwindHohLeft) {
                    let m = fsel(s + 1.0, s, -1.0);
                    -fsel(m, 0.0, m)
                } else {
                    let m = fsel(s, s, 0.0);
                    fsel(m - 1.0, 1.0, m)
                };
                let p = (side - k.hoh_start) / (1.0 - k.hoh_start);
                let p = fsel(p, p, 0.0);
                out.push(Cmd::Phase(fsel(p - 1.0, 1.0, p)));
            }
            // F-85600
            SlipHohLeft | SlipHohRight => {
                let s = c.smooth;
                let m = fsel(s + 1.0, s, -1.0);
                out.push(Cmd::Phase5((fsel(m - 1.0, 1.0, m) + 1.0) * 0.5));
            }
            // F-99138
            NosBoost => out.push(Cmd::Phase(half(c))),
            // F-83192: hold the horn at half way while honking
            HonkTheHorn => {
                let honk = c.flags & F_HONKING != 0;
                if e.phase > 0.5 && honk {
                    out.push(Cmd::Phase(0.5));
                    out.push(Cmd::Rate(0.0));
                } else if !honk {
                    out.push(Cmd::Rate(1.0));
                }
            }
            _ => {}
        }
    }

    /// The cruising update F-65297: the face (neutral, anger above 80 mph, alarmed while the sim says so; back to neutral
    /// under 70 mph), the steering sweep's weight and phase, the smoothing, timer B, the pan overlay.
    fn cruise(&mut self, c: &mut Ctx, k: &Tune, r: &Racer, out: &mut Vec<Cmd>) {
        let alarm = r.alarm > 0.5;
        let speed = c.speed;
        let mut face = None;
        if self.face_state == 0 {
            face = self.face_of("drv_express_Neutral", r);
            self.face_state = 1;
        }
        match self.face_state {
            1 => {
                if alarm {
                    face = self.face_of("drv_express_Alarmed", r);
                    self.face_state = 3;
                } else if speed > MPH80 {
                    face = self.face_of("drv_express_Anger", r);
                    self.face_state = 2;
                }
            }
            2 => {
                if alarm {
                    face = self.face_of("drv_express_Alarmed", r);
                    self.face_state = 3;
                } else if !(speed >= MPH70) {
                    face = self.face_of("drv_express_Neutral", r);
                    self.face_state = 1;
                }
            }
            3 if !alarm => {
                if speed > MPH80 {
                    face = self.face_of("drv_express_Anger", r);
                    self.face_state = 2;
                } else {
                    face = self.face_of("drv_express_Neutral", r);
                    self.face_state = 1;
                }
            }
            _ => {}
        }
        if face.is_some() {
            self.face = face;
            out.push(Cmd::Pair(STEER_SWEEP, STEER_SWEEP, false, 0.25));
            out.push(Cmd::Rate(0.0));
        }
        out.push(Cmd::Weight(if c.opponent { 0.0 } else { fast(speed) }));
        out.push(Cmd::Phase2(((c.steer_add + c.smooth) + 1.0) * 0.5));
        c.smooth_k = if c.skid > 0.51 { [10.0, 25.0] } else { k.cruise_k };
        if !(c.goal.abs() >= 0.95) {
            c.timer_b = 0.0;
        }
        if self.overlay.is_none() && c.timer_c > 0.75 {
            self.pan(r);
        }
        if c.flags & F_REVERSING != 0 {
            c.flags2 |= 8;
        }
    }

    fn enter(&mut self, c: &mut Ctx, k: &Tune, r: &Racer, out: &mut Vec<Cmd>) {
        use State::*;
        let play = |out: &mut Vec<Cmd>, clip: Clip| out.push(Cmd::Play(clip, 0.25, false, false));
        match self.state {
            // F-19928 (also suspended's), F-86219
            StoppedIdle | Suspended | StoppedIdleTimeup => {
                self.face = self.face_of("drv_express_Neutral", r);
                out.push(Cmd::Play(IDLE_STP, 0.25, true, false));
                c.timer_a = 0.0;
                self.random(c, 10.0, 20.0);
                if self.state == StoppedIdleTimeup {
                    c.flags2 |= 8;
                }
            }
            // F-27131
            Cruising => {
                self.face_state = 0;
                c.timer_a = 0.0;
                c.timer_b = 0.0;
                self.random(c, 10.0, 20.0);
                c.flags2 |= 0x20;
            }
            // F-63640 / F-42297
            WindHohLeft | WindHohRight => {
                play(out, if self.state == WindHohLeft { "DRV_INTO_STEERL_HOH" } else { "DRV_INTO_STEERR_HOH" });
                Self::rescale(c, k.hoh_steer);
                c.smooth_k = k.wind_k;
                out.push(Cmd::Rate(0.0));
            }
            // F-64076 / F-46182
            UnwindHohLeft | UnwindHohRight => {
                Self::rescale(c, k.hoh_steer);
                c.smooth_k = k.unwind_k;
                play(out, if self.state == UnwindHohLeft { "DRV_INTO_STEERL_HOH" } else { "DRV_INTO_STEERR_HOH" });
                out.push(Cmd::Rate(0.0));
            }
            // F-25419 / F-13166
            SlipHohLeft => {
                c.smooth_k = k.slip_k;
                let l = ["DRV_INTO_STEERL_HOH_90L", "DRV_INTO_STEERL_HOH_45L", "DRV_INTO_STEERL_HOH_00", "DRV_INTO_STEERL_HOH_45R", "DRV_INTO_STEERL_HOH_90R"];
                out.push(Cmd::Five(l, 0.125, false));
            }
            SlipHohRight => {
                c.smooth_k = k.slip_k;
                let l = ["DRV_INTO_STEERR_HOH_90L", "DRV_INTO_STEERR_HOH_45L", "DRV_INTO_STEERR_HOH_00", "DRV_INTO_STEERR_HOH_45R", "DRV_INTO_STEERR_HOH_90R"];
                out.push(Cmd::Five(l, 0.125, false));
            }
            // F-58126
            Shifting => {
                let l = ["DRV_STEERL_SFT_SHFT_90", "DRV_STEERL_SFT_SHFT_45", "DRV_SHFT", "DRV_STEERR_SFT_SHFT_45", "DRV_STEERR_SFT_SHFT_90"];
                out.push(Cmd::Five(l, 0.125, false));
                c.flags2 |= 0x20;
            }
            // F-23694
            IntoReverse => {
                self.overlay = None;
                play(out, REV);
            }
            // F-28459
            Reversing => {
                play(out, "DRV_REV_STEER");
                out.push(Cmd::Rate(0.0));
                Self::rescale(c, k.reverse_steer);
                c.timer_a = 0.0;
            }
            // F-61122: the reverse clip backwards from its end
            OutofReverse => {
                c.timer_c = 0.0;
                play(out, REV);
                out.push(Cmd::Phase(1.0));
                out.push(Cmd::Rate(-1.0));
            }
            // F-51895
            NosBoost => {
                self.face = self.face_of("drv_express_Alarmed", r);
                play(out, "DRV_NOS_STEER_SWEEP");
                out.push(Cmd::Rate(0.0));
            }
            // F-80337
            HonkTheHorn => {
                self.face = self.face_of("drv_express_Anger", r);
                out.push(Cmd::Play("drv_honk", 0.125, false, false));
                out.push(Cmd::Rate(1.0));
            }
            // F-20147
            ImpactSoft => {
                self.face = self.face_of("drv_express_Alarmed", r);
                play(out, "DRV_HIT_SFT");
                c.flags2 |= 4;
            }
            // F-38534
            ImpactDriveable | ImpactDamageout | ImpactDamageoutInverted => {
                play(out, "DRV_HIT_HRD");
                c.flags2 |= 4;
            }
            // F-80396: the engine starts unless the sim's +592 bit 5 says otherwise (then the driver is disappointed)
            ImpactStartCar => {
                c.timer_c = 0.0;
                if !r.engine_off {
                    out.push(Cmd::EngineStart);
                }
                play(out, if r.engine_off { "drv_gest_disapnt" } else { "DRV_IDLE_START" });
            }
            // F-28455 (its picker F-34469 steps the counter before it indexes)
            Fidget => {
                self.overlay = None;
                self.counters[7] = self.counters[7].wrapping_add(1);
                let clip = FIDGET[(self.counters[7] % 2) as usize];
                play(out, clip);
            }
            GestureSwitch => self.gesture(c, r, out),
            _ => {}
        }
    }

    /// A picker's round robin: counter `i` modulo `n` (the original's signed remainder), then the counter steps.
    fn next(&mut self, i: usize, n: i32) -> usize {
        let v = self.counters[i];
        self.counters[i] = v.wrapping_add(1);
        (v % n) as usize
    }

    /// The gesture switch's enter F-20767: the context's gesture (+4) picks the clip.
    fn gesture(&mut self, c: &mut Ctx, r: &Racer, out: &mut Vec<Cmd>) {
        self.overlay = None;
        let skip_right = r.mode3 && r.human;
        let clip: Option<Clip> = match c.gesture as u32 {
            // F-80898: one skip of the response the racer does not make
            0 => {
                let mut clip = NEUTRAL[self.next(0, 5)];
                if clip.contains(if skip_right { "_rspnd_rgt" } else { "_rspnd_lft" }) {
                    clip = NEUTRAL[self.next(0, 5)];
                }
                Some(clip)
            }
            1 => Some(LEFT[self.next(1, 5)]),
            2 => Some(RIGHT[self.next(2, 5)]),
            3 => Some(TALK_LEFT[self.next(3, 4)]),
            4 => Some(TALK_RIGHT[self.next(4, 4)]),
            5 => Some(MILD[self.next(5, 2)]),
            6 => {
                let clip = WIN[self.next(6, 2)];
                self.face = self.face_of("drv_express_Tense", r);
                Some(clip)
            }
            7 => Some("drv_gest_listen_lft"),
            8 => Some("drv_gest_listen_rgt"),
            9 => Some("drv_gest_thm_dwn_lft"),
            10 => Some("drv_gest_thm_dwn_rgt"),
            11 => Some("drv_gest_cas_rgt_01"),
            12 => Some("drv_gest_look_rgt"),
            13 => Some("drv_gest_mild_rgt_01"),
            15 => {
                self.face = self.face_of("drv_express_Anger", r);
                Some("Drv_Idle_Low")
            }
            17 => {
                self.counters[8] = self.counters[8].wrapping_add(1);
                Some("drv_gest_disapnt")
            }
            18 => Some("drv_gest_rspnd"),
            // the find_sh_04 cutscene's gesture: its clip and the cutscene mover's (F-42193) as the overlay
            19 => {
                self.find_sh = true;
                out.push(Cmd::PlayCut(FIND_SH, 0.25, false, false));
                out.push(Cmd::Overlay(FIND_SH_MOVER, 0.25, false));
                return;
            }
            _ => None,
        };
        if let Some(clip) = clip {
            out.push(Cmd::Play(clip, 0.25, false, false));
            if self.jawflap {
                out.push(Cmd::Overlay("DRV_GEST_JAWFLAP", 0.25, true));
            }
        }
    }

    fn exit(&mut self, c: &mut Ctx, k: &Tune, r: &Racer, out: &mut Vec<Cmd>) {
        use State::*;
        let finish = c.flags & F_RACE_FINISH != 0;
        match self.state {
            // F-64876
            StoppedIdleTimeup | IntoReverse | OutofReverse => c.flags2 &= !8,
            // F-26089
            Cruising | Shifting => c.flags2 &= !0x20,
            // F-37069
            Reversing => Self::rescale(c, k.start_steer),
            // F-17031
            WindHohLeft | UnwindHohLeft | WindHohRight | UnwindHohRight => {
                Self::rescale(c, k.start_steer);
                c.smooth_k = k.cruise_k;
            }
            // F-51463
            SlipHohLeft | SlipHohRight => c.smooth_k = k.cruise_k,
            // F-25128, F-70617 (the camera takes the racer back unless the race is over)
            ImpactSoft | ImpactDriveable | ImpactDamageout | ImpactDamageoutInverted => {
                c.flags2 &= !4;
                c.timer_c = 0.0;
                if self.state == ImpactDamageoutInverted && !finish {
                    out.push(Cmd::HandBack);
                }
            }
            // F-47123
            ImpactStartCar => {
                c.timer_c = 0.0;
                if !finish {
                    out.push(Cmd::HandBack);
                }
            }
            // F-85478
            ImpactStartCarDone => {
                c.flags2 &= !4;
                c.timer_c = 0.0;
                out.push(Cmd::EngineStop);
            }
            // F-16760 (also the fidget's): the pan overlay back, no face, the find_sh_04 idle, no gesture
            Fidget | GestureSwitch => {
                self.pan(r);
                self.face = None;
                if self.find_sh {
                    self.find_sh = false;
                    out.push(Cmd::Play(IDLE_STP, 0.0, true, true));
                }
                self.racer_684 = 20;
                c.gesture = 20;
                c.timer_a = 0.0;
            }
            _ => {}
        }
    }
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
                ..Ctx::default()
            };
            let by = |x: u32, i: u32| (x >> (24 - 8 * i)) & 0xff != 0;
            let k = Tune {
                enabled: [by(w[24], 0), by(w[24], 1), by(w[24], 2), by(w[24], 3), by(w[25], 0), by(w[25], 1)],
                bands: [f(26), f(27), f(28), f(29)],
                start_steer: f(30),
                hoh_skid_window: f(31),
                impact: [f(32), f(33), f(34)],
                ..Tune::default()
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

    /// The machine's step with its callbacks against the original's (its Init, the constructor's tune, then per frame the
    /// inputs written into the context and its step F-58452, the driver's anim player logged: `work/drvmachine/step_ref.py`).
    #[test]
    fn the_step_matches_the_original() {
        let lc = |s: &str| s.to_ascii_lowercase();
        let b = |x: f32| format!("{:08x}", x.to_bits());
        let cmd = |c: &Cmd| -> String {
            match c {
                Cmd::Play(a, t, x, y) => format!("play {} {} {} {}", lc(a), b(*t), *x as u8, *y as u8),
                Cmd::PlayCut(a, t, x, y) => format!("play2 {} {} {} {}", lc(a), b(*t), *x as u8, *y as u8),
                Cmd::Pair(a, z, x, t) => format!("pair {} {} {} {}", lc(a), lc(z), *x as u8, b(*t)),
                Cmd::Five(l, t, x) => format!("five {} {} {} {} {} {} {}", lc(l[0]), lc(l[1]), lc(l[2]), lc(l[3]), lc(l[4]), b(*t), *x as u8),
                Cmd::Phase(v) => format!("phase {}", b(*v)),
                Cmd::Rate(v) => format!("rate {}", b(*v)),
                Cmd::Rate5(v) => format!("rate5 {}", b(*v)),
                Cmd::Phase2(v) => format!("phase2 {}", b(*v)),
                Cmd::Phase5(v) => format!("phase5 {}", b(*v)),
                Cmd::PhaseAll(v) => format!("phaseall {}", b(*v)),
                Cmd::Weight(v) => format!("weight {}", b(*v)),
                Cmd::Overlay(a, t, x) => format!("overlay {} {} {}", lc(a), b(*t), *x as u8),
                Cmd::EngineStart => "engine56 1".into(),
                Cmd::EngineStop => "engine52 0".into(),
                Cmd::HandBack => "handback".into(),
            }
        };
        let want: Vec<&str> = include_str!("testdata/drvmachine_step_ref.txt").lines().collect();
        let mut got: Vec<String> = Vec::new();
        let (mut m, mut c, mut k) = (Machine::new("drv_mp_01_set", [0, 0]), Ctx::default(), Tune::original());
        let mut mode3 = false;
        let mut frames = 0;
        for line in include_str!("testdata/drvmachine_step_cases.txt").lines() {
            let w: Vec<u32> = line.split_whitespace().skip(line.starts_with('=') as usize).map(|x| u32::from_str_radix(x, 16).unwrap()).collect();
            if line.starts_with('=') {
                m = Machine::new("drv_mp_01_set", [w[0], w[1]]);
                c = Ctx { wheel_scale: 1.0, ..Ctx::default() };
                mode3 = w[2] == 3;
                let by = |x: u32, i: u32| (x >> (24 - 8 * i)) & 0xff != 0;
                k = Tune { enabled: [by(w[3], 0), by(w[3], 1), by(w[3], 2), by(w[3], 3), by(w[4], 0), by(w[4], 1)], ..Tune::original() };
                got.push("=".into());
                continue;
            }
            let f = |i: usize| f32::from_bits(w[i]);
            c.gesture = w[0] as i32;
            c.flags = w[1];
            c.goal = f(2);
            c.steer = f(3);
            c.smooth = f(4);
            c.to_limit = f(5);
            c.dsteer = f(6);
            c.culled = w[7] & 0xff != 0;
            c.opponent = w[8] & 0xff != 0;
            c.skid_timer = f(9);
            c.speed = f(10);
            c.shift = w[11] as i32;
            c.skid = f(12);
            c.steer_add = f(20);
            c.timer_a += f(13);
            c.timer_b += f(13);
            c.timer_c += f(13);
            let e = Env { remaining: f(14), phase: f(15), cutscene: w[16] != 0 };
            let r = Racer { human: w[17] & 0xff != 0, mode3, alarm: f(19), engine_off: w[18] & 0x20 != 0 };
            let mut out = Vec::new();
            m.step(&mut c, &k, &e, &r, &mut out);
            got.extend(out.iter().map(cmd));
            let name = STATES.iter().find(|(s, _)| *s == m.state).unwrap().1;
            got.push(format!(
                "S {name} {:08x} {} {} {} {} {} {} {} {} {} {} {} {} {} {}",
                c.flags2,
                b(c.steer),
                b(c.smooth),
                b(c.smooth_k[0]),
                b(c.smooth_k[1]),
                b(c.wheel_scale),
                b(c.timer_a),
                b(c.timer_b),
                b(c.timer_c),
                b(c.random),
                m.face.as_deref().map(lc).unwrap_or("-".into()),
                m.overlay.map(lc).unwrap_or("-".into()),
                m.face_state,
                c.gesture,
                m.find_sh as u8
            ));
            frames += 1;
        }
        for (i, (g, w)) in got.iter().zip(&want).enumerate() {
            assert_eq!(g, w, "output line {} ({} lines before: {:?})", i + 1, 6, &got[i.saturating_sub(6)..i]);
        }
        assert_eq!(got.len(), want.len());
        assert!(frames > 10_000, "{frames}");
    }
}
