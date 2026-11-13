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
    if slowtick().is_none() && !tickpts() {
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

// (MCLA_TICKPTS=1: the time between this tick's named points, summed by the pair and logged as means and maxima with
// the minute's ticktimes line - which part of a section costs what; docs/PERFORMANCE.md)
fn tickpts() -> bool {
    static V: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *V.get_or_init(|| std::env::var_os("MCLA_TICKPTS").is_some())
}

// (MCLA_TICKPTS: the game thread's waits for the audio mixer's lock this tick, and the minute's sum / worst tick)
thread_local!(static LOCK_WAIT: std::cell::Cell<(f64, f64, f64, u32)> = const { std::cell::Cell::new((0.0, 0.0, 0.0, 0)) });

/// The game thread waited `ms` for the audio mixer's lock (counted with MCLA_TICKPTS only).
pub fn lock_wait(ms: f64) {
    LOCK_WAIT.with(|w| {
        let mut v = w.get();
        v.0 += ms;
        v.3 += 1;
        w.set(v);
    });
}

/// Whether the mixer lock waits are counted.
pub fn counting() -> bool {
    tickpts()
}

thread_local!(static PT_SUM: RefCell<Vec<(&'static str, &'static str, f64, f64, u32)>> = const { RefCell::new(Vec::new()) });

// (this tick's points into the sums: each span from the point before it, the first from the tick's start)
fn sum_points() {
    PTS.with(|p| {
        let p = p.borrow();
        PT_SUM.with(|s| {
            let mut s = s.borrow_mut();
            let mut prev: (&'static str, f64) = ("tick start", 0.0);
            for &(name, ms) in &p.1 {
                let d = ms - prev.1;
                match s.iter_mut().find(|e| e.0 == prev.0 && e.1 == name) {
                    Some(e) => {
                        e.2 += d;
                        e.3 = e.3.max(d);
                        e.4 += 1;
                    }
                    None => s.push((prev.0, name, d, d, 1)),
                }
                prev = (name, ms);
            }
        });
    });
}

/// The tick begins (its time goes to "other" until the first mark).
pub fn start() {
    let now = Instant::now();
    T_.with(|t| t.borrow_mut().at = Some((S::Other, now)));
    if slowtick().is_some() || tickpts() {
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
        if tickpts() {
            sum_points();
            LOCK_WAIT.with(|w| {
                let mut v = w.get();
                v.1 += v.0;
                v.2 = v.2.max(v.0);
                v.0 = 0.0;
                w.set(v);
            });
        }
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
        if tickpts() {
            PT_SUM.with(|s| {
                let mut s = s.borrow_mut();
                let list: Vec<String> = s.iter().map(|e| format!("{} -> {} {:.3} max {:.2} ({})", e.0, e.1, e.2 / TICKS as f64, e.3, e.4)).collect();
                log::info!("tickpts: means a tick over {} ticks: {}", TICKS, list.join(" | "));
                LOCK_WAIT.with(|w| {
                    let v = w.get();
                    log::info!("tickpts: mixer lock waits {:.3} ms a tick, worst tick {:.2} ms, {} locks a tick", v.1 / TICKS as f64, v.2, v.3 / TICKS);
                    w.set((0.0, 0.0, 0.0, 0));
                });
                s.clear();
            });
        }
        t.sum = [0.0; 11];
        t.ticks = 0;
    });
}
