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
}

thread_local!(static T_: RefCell<T> = RefCell::new(T { at: None, sum: [0.0; 11], ticks: 0 }));

/// The tick begins (its time goes to "other" until the first mark).
pub fn start() {
    let now = Instant::now();
    T_.with(|t| t.borrow_mut().at = Some((S::Other, now)));
}

/// From here on the tick's time goes to `s`.
pub fn mark(s: S) {
    let now = Instant::now();
    T_.with(|t| {
        let mut t = t.borrow_mut();
        if let Some((prev, at)) = t.at.replace((s, now)) {
            t.sum[prev as usize] += (now - at).as_secs_f64() * 1000.0;
        }
    });
}

/// The tick is over; `level_objs`: the physics level's object count (logged with the means).
pub fn end(level_objs: usize) {
    let now = Instant::now();
    T_.with(|t| {
        let mut t = t.borrow_mut();
        let Some((prev, at)) = t.at.take() else { return };
        t.sum[prev as usize] += (now - at).as_secs_f64() * 1000.0;
        t.ticks += 1;
        if t.ticks < TICKS {
            return;
        }
        let total: f64 = t.sum.iter().sum();
        let mut parts: Vec<(usize, f64)> = t.sum.iter().copied().enumerate().collect();
        parts.sort_by(|a, b| b.1.total_cmp(&a.1));
        let list: Vec<String> = parts.iter().map(|(i, ms)| format!("{} {:.2}", NAMES[*i], ms / TICKS as f64)).collect();
        log::info!("ticktimes: {:.2} ms a tick ({} ticks; physics level {} objects): {}", total / TICKS as f64, TICKS, level_objs, list.join(", "));
        t.sum = [0.0; 11];
        t.ticks = 0;
    });
}
