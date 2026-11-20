//! A census of the heap by call site, a test hook for GitHub #48's memory creep (private memory rising ~40 MB a minute in
//! free roam, v0.21.0 and v0.22.0 alike; docs/PERFORMANCE.md). `MCLA_HEAP_CENSUS=<s>[,<min KB>]`: every allocation of
//! at least min KB (default 1024) has its call stack taken (RtlCaptureStackBackTrace, as offsets into the exe) and is
//! counted on its site until it is freed; every <s> seconds the log gets the live total and the sites holding most
//! (`heapcensus:` lines; `tools/heapcensus.py <exe> <log>` turns the offsets into functions). A third number,
//! `MCLA_HEAP_CENSUS=<s>,0,<N>`, takes min KB 0 and samples the blocks under 1 KB: one address in N (by a hash of the
//! address, so a block's free finds it) is tracked, counted N times. With the census on, every allocation of any size
//! is also added to its size class (the `all:` line): the Rust heap's whole live total, to tell it from memory the
//! process holds outside it (drivers, the system's own heaps); allocations made before the census started and freed
//! later can make a class a little low. Off (the default) the allocator is the system's with one relaxed load in front:
//! the game's memory and output are the same.
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, AtomicUsize, Ordering::Relaxed};
use std::sync::Mutex;

pub struct Census;

static ON: AtomicBool = AtomicBool::new(false);
static MIN: AtomicUsize = AtomicUsize::new(1 << 20);
static EVERY: AtomicU64 = AtomicU64::new(60);
static LAST: AtomicU64 = AtomicU64::new(0);
static BASE: AtomicUsize = AtomicUsize::new(0);
static SAMPLE: AtomicUsize = AtomicUsize::new(1);
const FRAMES: usize = 24;
/// the sites a report lists (`MCLA_HEAP_CENSUS=<s>,<min KB>,<N>,<sites>`; default 25)
static TOP: AtomicUsize = AtomicUsize::new(25);
/// the size classes' upper bounds (bytes) of the `all:` line; the last class is everything larger
const CLASS: [usize; 5] = [64, 256, 1024, 16 << 10, 256 << 10];
static CLASS_LIVE: [AtomicI64; 6] = [const { AtomicI64::new(0) }; 6];
static CLASS_N: [AtomicI64; 6] = [const { AtomicI64::new(0) }; 6];

fn class_add(size: usize, sign: i64) {
    let c = CLASS.iter().position(|&b| size < b).unwrap_or(CLASS.len());
    CLASS_LIVE[c].fetch_add(sign * size as i64, Relaxed);
    CLASS_N[c].fetch_add(sign, Relaxed);
}

/// The weight a block of `size` at `p` is counted with: 0 = not tracked, N = one of the 1-in-N sampled small blocks.
fn weight(p: *mut u8, size: usize) -> usize {
    if size < MIN.load(Relaxed) {
        return 0;
    }
    let n = SAMPLE.load(Relaxed);
    if size >= 1024 || n <= 1 {
        return 1;
    }
    // (heap blocks are 16-aligned: hash the rest of the address)
    if (((p as usize >> 4) as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 32) as usize % n == 0 { n } else { 0 }
}

struct Site {
    frames: [usize; FRAMES],
    n: usize,
    live: usize,
    blocks: usize,
    made: u64,
}

#[derive(Default)]
struct State {
    blocks: HashMap<usize, (u32, usize, usize)>,
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
    fn GetProcessHeap() -> *mut c_void;
    fn GetProcessHeaps(n: u32, out: *mut *mut c_void) -> u32;
    fn HeapSummary(heap: *mut c_void, flags: u32, out: *mut HeapSum) -> i32;
    fn HeapCompact(heap: *mut c_void, flags: u32) -> usize;
}

/// `MCLA_HEAP_COMPACT=1` with the census: each report also runs HeapCompact on the process heap (coalesces its free
/// blocks and gives large free runs back to the system) and logs the heaps before and after and the call's time. A
/// test of whether the committed-but-free part of the heap can be returned; the game's allocations are not touched.
#[cfg(windows)]
fn compact_line() -> Option<String> {
    if std::env::var("MCLA_HEAP_COMPACT").map_or(true, |v| v != "1") {
        return None;
    }
    let t = std::time::Instant::now();
    let largest = unsafe { HeapCompact(GetProcessHeap(), 0) };
    Some(format!("compact took {:.1} ms (largest free block {:.1} MB), after: {}", t.elapsed().as_secs_f64() * 1000.0, largest as f64 / 1048576.0, heaps_line()))
}

#[cfg(not(windows))]
fn compact_line() -> Option<String> {
    None
}

#[cfg(windows)]
#[repr(C)]
#[derive(Default)]
struct HeapSum {
    cb: u32,
    allocated: usize,
    committed: usize,
    reserved: usize,
    max_reserve: usize,
}

/// The heaps' line: the process heap (the Rust allocator's) allocated / committed, and the process's other heaps
/// (drivers', runtimes') together; committed minus allocated is free space the heap keeps.
#[cfg(windows)]
fn heaps_line() -> String {
    let mut hs = [std::ptr::null_mut::<c_void>(); 256];
    let n = unsafe { GetProcessHeaps(hs.len() as u32, hs.as_mut_ptr()) } as usize;
    let me = unsafe { GetProcessHeap() };
    let (mut pa, mut pc, mut oa, mut oc) = (0usize, 0usize, 0usize, 0usize);
    for &h in &hs[..n.min(hs.len())] {
        let mut s = HeapSum { cb: std::mem::size_of::<HeapSum>() as u32, ..Default::default() };
        if unsafe { HeapSummary(h, 0, &mut s) } == 0 {
            continue;
        }
        if h == me { pa += s.allocated; pc += s.committed } else { oa += s.allocated; oc += s.committed }
    }
    let mb = |x: usize| x as f64 / 1048576.0;
    format!("process heap {:.1} MB allocated, {:.1} committed; {} other heaps {:.1} allocated, {:.1} committed", mb(pa), mb(pc), n.saturating_sub(1), mb(oa), mb(oc))
}

#[cfg(not(windows))]
fn heaps_line() -> String {
    String::new()
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
    if p.is_null() {
        return;
    }
    class_add(size, 1);
    let w = weight(p, size);
    if w == 0 {
        return;
    }
    guarded(|| {
        let mut frames = [0usize; FRAMES];
        let (hash, n) = stack(&mut frames);
        let mut g = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let st = g.get_or_insert_with(State::default);
        let site = st.sites.entry(hash).or_insert(Site { frames, n, live: 0, blocks: 0, made: 0 });
        site.live += size * w;
        site.blocks += w;
        site.made += w as u64;
        st.blocks.insert(p as usize, (hash, size, w));
    });
}

fn untrack(p: *mut u8, size: usize) {
    class_add(size, -1);
    if weight(p, size) == 0 {
        return;
    }
    guarded(|| {
        let mut g = STATE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(st) = g.as_mut() {
            if let Some((hash, sz, w)) = st.blocks.remove(&(p as usize)) {
                if let Some(s) = st.sites.get_mut(&hash) {
                    s.live -= sz * w;
                    s.blocks -= w;
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
    let min_kb = it.next().and_then(|s| s.parse::<usize>().ok()).unwrap_or(1024);
    let sample = it.next().and_then(|s| s.parse::<usize>().ok()).unwrap_or(1).max(1);
    TOP.store(it.next().and_then(|s| s.parse::<usize>().ok()).unwrap_or(25).max(1), Relaxed);
    EVERY.store(every, Relaxed);
    SAMPLE.store(sample, Relaxed);
    MIN.store(min_kb << 10, Relaxed);
    #[cfg(windows)]
    BASE.store(unsafe { GetModuleHandleW(std::ptr::null()) } as usize, Relaxed);
    LAST.store(now_s(), Relaxed);
    ON.store(true, Relaxed);
    log::info!("heapcensus: on, blocks of {min_kb} KB and more (under 1 KB one in {sample}), a report every {every} s");
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
        v.truncate(TOP.load(Relaxed));
        (st.sites.values().map(|s| s.live).sum::<usize>(), st.blocks.values().map(|b| b.2).sum::<usize>(), st.sites.len(), v)
    }) else {
        return;
    };
    let mb = |c: usize| CLASS_LIVE[c].load(Relaxed) as f64 / 1048576.0;
    let all: f64 = (0..6).map(mb).sum();
    let n: i64 = (0..6).map(|c| CLASS_N[c].load(Relaxed)).sum();
    log::info!(
        "heapcensus: t {t} s: all: {all:.1} MB live in {n} blocks (< 64 B {:.1}, < 256 B {:.1}, < 1 KB {:.1}, < 16 KB {:.1}, < 256 KB {:.1}, larger {:.1})",
        mb(0), mb(1), mb(2), mb(3), mb(4), mb(5)
    );
    if let Some(h) = guarded(heaps_line) {
        log::info!("heapcensus: t {t} s: heaps: {h}");
    }
    if let Some(Some(c)) = guarded(compact_line) {
        log::info!("heapcensus: t {t} s: heaps: {c}");
    }
    log::info!("heapcensus: t {t} s: {:.1} MB live in {blocks} blocks of >= {} KB, {sites} sites", live as f64 / 1048576.0, MIN.load(Relaxed) >> 10);
    for (i, (l, b, m, h, fr)) in top.iter().enumerate() {
        log::info!("heapcensus: #{i} site {h:08x}: {:.1} MB live in {b} blocks ({m} made) at {fr}", *l as f64 / 1048576.0);
    }
}
