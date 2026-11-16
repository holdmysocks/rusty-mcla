//! A census of the heap by call site, a test hook for GitHub #48's memory creep (private memory rising ~40 MB a minute in
//! free roam, v0.21.0 and v0.22.0 alike; docs/PERFORMANCE.md). `MCLA_HEAP_CENSUS=<s>[,<min KB>]`: every allocation of
//! at least min KB (default 1024) has its call stack taken (RtlCaptureStackBackTrace, as offsets into the exe) and is
//! counted on its site until it is freed; every <s> seconds the log gets the live total and the sites holding most
//! (`heapcensus:` lines; `tools/heapcensus.py <exe> <log>` turns the offsets into functions). Off (the default) the
//! allocator is the system's with one relaxed load in front: the game's memory and output are the same.
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering::Relaxed};
use std::sync::Mutex;

pub struct Census;

static ON: AtomicBool = AtomicBool::new(false);
static MIN: AtomicUsize = AtomicUsize::new(1 << 20);
static EVERY: AtomicU64 = AtomicU64::new(60);
static LAST: AtomicU64 = AtomicU64::new(0);
static BASE: AtomicUsize = AtomicUsize::new(0);
const FRAMES: usize = 24;
const TOP: usize = 15;

struct Site {
    frames: [usize; FRAMES],
    n: usize,
    live: usize,
    blocks: usize,
    made: u64,
}

#[derive(Default)]
struct State {
    blocks: HashMap<usize, (u32, usize)>,
    sites: HashMap<u32, Site>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

thread_local! {
    /// this thread is in the census's own bookkeeping: its allocations are not counted (and cannot lock STATE again)
    static BUSY: Cell<bool> = const { Cell::new(false) };
}

#[cfg(windows)]
#[link(name = "kernel32")]
extern "system" {
    fn RtlCaptureStackBackTrace(skip: u32, n: u32, out: *mut *mut c_void, hash: *mut u32) -> u16;
    fn GetModuleHandleW(name: *const u16) -> *mut c_void;
}

/// Run `f` as the census's own work on this thread; None when it already is (or the thread is going away).
fn guarded<R>(f: impl FnOnce() -> R) -> Option<R> {
    let entered = BUSY.try_with(|b| if b.get() { false } else { b.set(true); true }).unwrap_or(false);
    if !entered {
        return None;
    }
    let r = f();
    let _ = BUSY.try_with(|b| b.set(false));
    Some(r)
}

#[cfg(windows)]
fn stack(frames: &mut [usize; FRAMES]) -> (u32, usize) {
    let mut raw = [std::ptr::null_mut::<c_void>(); FRAMES];
    let mut hash = 0u32;
    // (skip this function, track and the allocator's own frame)
    let n = unsafe { RtlCaptureStackBackTrace(3, FRAMES as u32, raw.as_mut_ptr(), &mut hash) } as usize;
    let base = BASE.load(Relaxed);
    for i in 0..n {
        frames[i] = (raw[i] as usize).wrapping_sub(base);
    }
    (hash, n)
}

#[cfg(not(windows))]
fn stack(_frames: &mut [usize; FRAMES]) -> (u32, usize) {
    (0, 0)
}

fn track(p: *mut u8, size: usize) {
    if p.is_null() || size < MIN.load(Relaxed) {
        return;
    }
    guarded(|| {
        let mut frames = [0usize; FRAMES];
        let (hash, n) = stack(&mut frames);
        let mut g = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let st = g.get_or_insert_with(State::default);
        let site = st.sites.entry(hash).or_insert(Site { frames, n, live: 0, blocks: 0, made: 0 });
        site.live += size;
        site.blocks += 1;
        site.made += 1;
        st.blocks.insert(p as usize, (hash, size));
    });
}

fn untrack(p: *mut u8, size: usize) {
    if size < MIN.load(Relaxed) {
        return;
    }
    guarded(|| {
        let mut g = STATE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(st) = g.as_mut() {
            if let Some((hash, sz)) = st.blocks.remove(&(p as usize)) {
                if let Some(s) = st.sites.get_mut(&hash) {
                    s.live -= sz;
                    s.blocks -= 1;
                }
            }
        }
    });
}

unsafe impl GlobalAlloc for Census {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let p = System.alloc(l);
        if ON.load(Relaxed) {
            track(p, l.size());
        }
        p
    }

    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
        let p = System.alloc_zeroed(l);
        if ON.load(Relaxed) {
            track(p, l.size());
        }
        p
    }

    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        if ON.load(Relaxed) {
            untrack(p, l.size());
        }
        System.dealloc(p, l)
    }

    unsafe fn realloc(&self, p: *mut u8, l: Layout, new: usize) -> *mut u8 {
        let q = System.realloc(p, l, new);
        if !q.is_null() && ON.load(Relaxed) {
            untrack(p, l.size());
            track(q, new);
        }
        q
    }
}

fn now_s() -> u64 {
    static T0: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    T0.get_or_init(std::time::Instant::now).elapsed().as_secs()
}

/// At start (run_game): reads MCLA_HEAP_CENSUS and turns the census on.
pub fn init() {
    let Ok(v) = std::env::var("MCLA_HEAP_CENSUS") else { return };
    let mut it = v.split(',').map(str::trim);
    let every = it.next().and_then(|s| s.parse::<u64>().ok()).unwrap_or(60).max(1);
    let min_kb = it.next().and_then(|s| s.parse::<usize>().ok()).unwrap_or(1024).max(1);
    EVERY.store(every, Relaxed);
    MIN.store(min_kb << 10, Relaxed);
    #[cfg(windows)]
    BASE.store(unsafe { GetModuleHandleW(std::ptr::null()) } as usize, Relaxed);
    LAST.store(now_s(), Relaxed);
    ON.store(true, Relaxed);
    log::info!("heapcensus: on, blocks of {min_kb} KB and more, a report every {every} s");
}

/// Each frame: the report when it is due.
pub fn tick() {
    if !ON.load(Relaxed) {
        return;
    }
    let t = now_s();
    if t < LAST.load(Relaxed) + EVERY.load(Relaxed) {
        return;
    }
    LAST.store(t, Relaxed);
    // (copied out under the guard: the log's own allocations must not wait on STATE)
    let Some((live, blocks, sites, top)) = guarded(|| {
        let g = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(st) = g.as_ref() else { return (0, 0, 0, Vec::new()) };
        let mut v: Vec<(usize, usize, u64, u32, String)> = st
            .sites
            .iter()
            .filter(|(_, s)| s.live > 0)
            .map(|(h, s)| (s.live, s.blocks, s.made, *h, s.frames[..s.n].iter().map(|f| format!("{f:x}")).collect::<Vec<_>>().join(",")))
            .collect();
        v.sort_unstable_by(|a, b| b.0.cmp(&a.0));
        v.truncate(TOP);
        (st.sites.values().map(|s| s.live).sum::<usize>(), st.blocks.len(), st.sites.len(), v)
    }) else {
        return;
    };
    log::info!("heapcensus: t {t} s: {:.1} MB live in {blocks} blocks of >= {} KB, {sites} sites", live as f64 / 1048576.0, MIN.load(Relaxed) >> 10);
    for (i, (l, b, m, h, fr)) in top.iter().enumerate() {
        log::info!("heapcensus: #{i} site {h:08x}: {:.1} MB live in {b} blocks ({m} made) at {fr}", *l as f64 / 1048576.0);
    }
}
