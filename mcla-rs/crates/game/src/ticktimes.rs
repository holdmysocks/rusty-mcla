//! Where a game tick's time goes, for the log: the main sections of `update_gameplay`, timed between
//! marks and logged as means once every 3600 ticks (a minute of play), with the size of the cars' shared
//! physics level. A session that slows down as it goes on (GitHub #17) then names the part that grows.
//! The cost is a clock read per mark (~12 a tick).
use std::cell::RefCell;
use std::time::Instant;

#[derive(Clone, Copy)]
pub enum S {
    Other,
    World,
    Hangouts,
    Traffic,
    RaceScript,
    Vehicles,
    Props,
    Roamers,
    Particles,
    Police,
    Sound,
}

const NAMES: [&str; 11] = ["other", "world", "hangouts", "traffic", "race script", "vehicles", "props", "roamers", "particles", "police", "sound"];
const TICKS: u32 = 3600;

struct T {
    at: Option<(S, Instant)>,
    sum: [f64; 11],
    ticks: u32,
    // (this tick's sections, for MCLA_SLOWTICK)
    one: [f64; 11],
}

thread_local!(static T_: RefCell<T> = RefCell::new(T { at: None, sum: [0.0; 11], ticks: 0, one: [0.0; 11] }));

// (named points of this tick, ms from its start - only with MCLA_SLOWTICK, logged with a slow tick)
thread_local!(static PTS: RefCell<(Option<Instant>, Vec<(&'static str, f64)>)> = const { RefCell::new((None, Vec::new())) });

/// A named point in this tick (MCLA_SLOWTICK: a slow tick's line lists them with their time from the tick's start).
pub fn pt(name: &'static str) {
    if slowtick().is_none() {
        return;
    }
    let now = Instant::now();
    PTS.with(|p| {
        let mut p = p.borrow_mut();
        if let Some(t0) = p.0 {
            p.1.push((name, (now - t0).as_secs_f64() * 1000.0));
        }
    });
}

// (MCLA_SLOWTICK=<ms>: a tick longer than that is logged with its sections - docs/PERFORMANCE.md)
fn slowtick() -> Option<f64> {
    static V: std::sync::OnceLock<Option<f64>> = std::sync::OnceLock::new();
    *V.get_or_init(|| std::env::var("MCLA_SLOWTICK").ok().and_then(|v| v.parse().ok()))
}

/// The tick begins (its time goes to "other" until the first mark).
pub fn start() {
    let now = Instant::now();
    T_.with(|t| t.borrow_mut().at = Some((S::Other, now)));
    if slowtick().is_some() {
        PTS.with(|p| {
            let mut p = p.borrow_mut();
            p.0 = Some(now);
            p.1.clear();
        });
    }
}

/// From here on the tick's time goes to `s`.
pub fn mark(s: S) {
    let now = Instant::now();
    T_.with(|t| {
        let mut t = t.borrow_mut();
        if let Some((prev, at)) = t.at.replace((s, now)) {
            let ms = (now - at).as_secs_f64() * 1000.0;
            t.sum[prev as usize] += ms;
            t.one[prev as usize] += ms;
        }
    });
}

thread_local! {
    static CENSUS: std::cell::Cell<[usize; 7]> = const { std::cell::Cell::new([0; 7]) };
}

/// The physics level's objects by kind this tick (`LooseState::census`), logged with the means.
pub fn census(n: [usize; 7]) {
    CENSUS.with(|c| c.set(n));
}

/// The tick is over; `level_objs`: the physics level's object count (logged with the means).
pub fn end(level_objs: usize) {
    let now = Instant::now();
    T_.with(|t| {
        let mut t = t.borrow_mut();
        let Some((prev, at)) = t.at.take() else { return };
        let ms = (now - at).as_secs_f64() * 1000.0;
        t.sum[prev as usize] += ms;
        t.one[prev as usize] += ms;
        if let Some(lim) = slowtick() {
            let total: f64 = t.one.iter().sum();
            if total > lim {
                let list: Vec<String> = t.one.iter().enumerate().filter(|(_, ms)| **ms >= 0.5).map(|(i, ms)| format!("{} {:.1}", NAMES[i], ms)).collect();
                let pts = PTS.with(|p| p.borrow().1.iter().map(|(n, ms)| format!("{n} {ms:.1}")).collect::<Vec<_>>().join(", "));
                log::info!("slowtick: {:.1} ms: {}{}", total, list.join(", "), if pts.is_empty() { String::new() } else { format!(" | at {pts}") });
            }
        }
        t.one = [0.0; 11];
        t.ticks += 1;
        if t.ticks < TICKS {
            return;
        }
        let total: f64 = t.sum.iter().sum();
        let mut parts: Vec<(usize, f64)> = t.sum.iter().copied().enumerate().collect();
        parts.sort_by(|a, b| b.1.total_cmp(&a.1));
        let list: Vec<String> = parts.iter().map(|(i, ms)| format!("{} {:.2}", NAMES[*i], ms / TICKS as f64)).collect();
        let c = CENSUS.with(|c| c.get());
        log::info!(
            "ticktimes: {:.2} ms a tick ({} ticks; physics level {} objects - props {} untouched / {} moved, pieces {}, vehicles {} here / {} not, ambient {}, slots {}): {}",
            total / TICKS as f64, TICKS, level_objs, c[0], c[1], c[2], c[3], c[4], c[5], c[6], list.join(", ")
        );
        t.sum = [0.0; 11];
        t.ticks = 0;
    });
}
