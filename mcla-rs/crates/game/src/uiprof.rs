//! The UI's per-frame parts timed (test hook MCLA_UI_PROF=1): `mark(name)` closes the part since the previous mark,
//! `frame_end` logs the mean of each part every 240 frames. Off, a mark is one relaxed load.

use std::cell::RefCell;
use std::sync::OnceLock;
use std::time::Instant;

fn on() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("MCLA_UI_PROF").is_some())
}

struct Acc {
    last: Option<Instant>,
    parts: Vec<(&'static str, f64)>,
    frames: u32,
}

thread_local!(static ACC: RefCell<Acc> = const { RefCell::new(Acc { last: None, parts: Vec::new(), frames: 0 }) });

/// the frame's UI starts here
pub fn start() {
    if on() {
        ACC.with(|a| a.borrow_mut().last = Some(Instant::now()));
    }
}

/// the part since the previous mark is `name`'s
pub fn mark(name: &'static str) {
    if !on() {
        return;
    }
    let now = Instant::now();
    ACC.with(|a| {
        let mut a = a.borrow_mut();
        let Some(last) = a.last.replace(now) else { return };
        let ms = (now - last).as_secs_f64() * 1000.0;
        match a.parts.iter_mut().find(|p| p.0 == name) {
            Some(p) => p.1 += ms,
            None => a.parts.push((name, ms)),
        }
    });
}

/// the frame's UI is over: every 240 frames the means are logged
pub fn frame_end() {
    if !on() {
        return;
    }
    ACC.with(|a| {
        let mut a = a.borrow_mut();
        a.last = None;
        a.frames += 1;
        if a.frames >= 240 {
            let n = a.frames as f64;
            let line: Vec<String> = a.parts.iter().map(|(k, v)| format!("{k} {:.3}", v / n)).collect();
            log::info!("uiprof: {} (ms, mean of {} frames)", line.join(", "), a.frames);
            a.parts.clear();
            a.frames = 0;
        }
    });
}
