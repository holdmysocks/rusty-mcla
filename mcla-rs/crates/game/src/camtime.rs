//! The player camera's cost per frame, for the log (MCLA_CAMTIME=1; D59 cost audit): the camera block of `render`
//! timed in parts between marks, logged every 600 frames as mean and worst per part. Off, a mark is one load of a
//! cached flag.
use std::cell::RefCell;
use std::sync::OnceLock;
use std::time::Instant;

const NAMES: [&str; 5] = ["probe level", "car hulls", "views (boom, switch, look back)", "wipeout + event chooser", "ability, collide, shake, idle"];
const FRAMES: u32 = 600;

struct T {
    at: Option<(usize, Instant)>,
    sum: [f64; 5],
    max: [f64; 5],
    total_sum: f64,
    total_max: f64,
    one: [f64; 5],
    frames: u32,
}

thread_local!(static T_: RefCell<T> = RefCell::new(T { at: None, sum: [0.0; 5], max: [0.0; 5], total_sum: 0.0, total_max: 0.0, one: [0.0; 5], frames: 0 }));

fn on() -> bool {
    static V: OnceLock<bool> = OnceLock::new();
    *V.get_or_init(|| std::env::var_os("MCLA_CAMTIME").is_some())
}

/// The camera block begins (the probe level is built); its time goes to part 0.
pub fn begin() {
    if !on() {
        return;
    }
    T_.with(|t| {
        let mut t = t.borrow_mut();
        t.one = [0.0; 5];
        t.at = Some((0, Instant::now()));
    });
}

/// From here on the time goes to part `p`.
pub fn mark(p: usize) {
    if !on() {
        return;
    }
    T_.with(|t| {
        let mut t = t.borrow_mut();
        let now = Instant::now();
        if let Some((q, s)) = t.at {
            t.one[q] += (now - s).as_secs_f64() * 1000.0;
        }
        t.at = Some((p, now));
    });
}

/// The camera block ends (a frame without a begin is not counted).
pub fn end() {
    if !on() {
        return;
    }
    T_.with(|t| {
        let mut t = t.borrow_mut();
        let Some((q, s)) = t.at.take() else { return };
        t.one[q] += s.elapsed().as_secs_f64() * 1000.0;
        let total: f64 = t.one.iter().sum();
        for i in 0..5 {
            t.sum[i] += t.one[i];
            t.max[i] = t.max[i].max(t.one[i]);
        }
        t.total_sum += total;
        t.total_max = t.total_max.max(total);
        t.frames += 1;
        if t.frames >= FRAMES {
            let n = t.frames as f64;
            let parts: Vec<String> = (0..5).map(|i| format!("{} {:.3} (max {:.2})", NAMES[i], t.sum[i] / n, t.max[i])).collect();
            log::info!("camtime: {:.3} ms a frame (max {:.2}) over {} frames: {}", t.total_sum / n, t.total_max, t.frames, parts.join(", "));
            *t = T { at: None, sum: [0.0; 5], max: [0.0; 5], total_sum: 0.0, total_max: 0.0, one: [0.0; 5], frames: 0 };
        }
    });
}
