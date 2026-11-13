//! `MCLA_PASS_TIMES=1`: the CPU time of recording each part of the frame's GPU passes (main.rs `render`, between the UI
//! and the submission), averaged over 240 frames and logged as `passes:` (docs/PERFORMANCE.md).

use std::cell::RefCell;
use std::time::Instant;

struct Prof {
    on: bool,
    last: Option<(Instant, &'static str)>,
    sums: Vec<(&'static str, f64)>,
    frames: u32,
}

thread_local! {
    static P: RefCell<Prof> = RefCell::new(Prof { on: std::env::var("MCLA_PASS_TIMES").is_ok(), last: None, sums: Vec::new(), frames: 0 });
}

/// The part `name` starts here (the one before ends).
pub fn mark(name: &'static str) {
    P.with(|p| {
        let mut p = p.borrow_mut();
        if !p.on {
            return;
        }
        let now = Instant::now();
        if let Some((t, n)) = p.last.take() {
            let ms = (now - t).as_secs_f64() * 1000.0;
            match p.sums.iter_mut().find(|s| s.0 == n) {
                Some(s) => s.1 += ms,
                None => p.sums.push((n, ms)),
            }
        }
        p.last = Some((now, name));
    });
}

/// The shadow block's side-by-side jobs (main.rs: 0 the shadow map, 1 - 4 the cascades, 5 the collector, 6 the
/// reflection, 7 the night collector, 8 the walkers' and the cast's skinning): each job's own time, summed over the 240
/// frames of a `passes:` line and logged beside it as `jobs:`.
static JOBS: std::sync::Mutex<[f64; 12]> = std::sync::Mutex::new([0.0; 12]);

/// Whether the pass times are on (MCLA_PASS_TIMES).
pub fn on() -> bool {
    P.with(|p| p.borrow().on)
}

/// Job `j` took `ms` this frame.
pub fn job(j: usize, ms: f64) {
    if let Ok(mut a) = JOBS.lock() {
        a[j.min(11)] += ms;
    }
}

/// The frame's passes end here.
pub fn end() {
    mark("");
    P.with(|p| {
        let mut p = p.borrow_mut();
        if !p.on {
            return;
        }
        p.last = None;
        p.frames += 1;
        if p.frames == 240 {
            let line: Vec<String> = p.sums.iter().map(|(n, s)| format!("{n} {:.2}", s / 240.0)).collect();
            log::info!("passes: {} ms", line.join(", "));
            if let Ok(mut a) = JOBS.lock() {
                let names = ["shadow", "casc1", "casc2", "casc3", "casc4", "collector", "reflection", "night", "skinning", "casc sectors", "casc cars+peds", "casc props"];
                let l: Vec<String> = names.iter().zip(a.iter()).map(|(n, v)| format!("{n} {:.2}", v / 240.0)).collect();
                log::info!("jobs: {} ms", l.join(", "));
                *a = [0.0; 12];
            }
            p.sums.clear();
            p.frames = 0;
        }
    });
}
