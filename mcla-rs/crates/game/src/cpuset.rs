//! The game's threads on a hybrid CPU (D59.pcores, docs/PERFORMANCE.md): the processor's core classes are logged at
//! start, the process is taken out of Windows' power throttling (EcoQoS, which lowers clocks and moves threads to the
//! efficiency cores), and - only when the CPU has more than one core class - the worker pool is made as many threads as
//! the fastest class has logical processors, its threads and the main thread kept on those. A machine with one core
//! class (most desktops, the 4-core machines) keeps the default pool and placement. Only where threads run changes,
//! never what they compute: the output is the same.
//!
//! A user's laptop (Core Ultra 9 285H: 6 performance, 8 efficiency and 2 low-power cores, v0.21.0) had pass recording
//! 11.3 ms and prep 3.3 ms a frame - the phases that wait on worker threads - where this 7900X has 3.8 and 1.6 with the
//! same game ticks (~2 ms): the joins waited on threads on the slow cores. MCLA_CPUSET=off: none of this (the default
//! pool, throttling as Windows decides); MCLA_CPUSET=nothrottle: only the throttling opt-out; MCLA_CPUSET=test<n>: the
//! first n processors taken as the fast class (the hybrid set-up tried on a machine with one class).

/// Logs the core classes and sets the pool and placement up. Call once at start, before the first rayon use.
pub fn init() {
    let mode = std::env::var("MCLA_CPUSET").unwrap_or_default();
    if mode == "off" {
        log::info!("cpu: MCLA_CPUSET=off - default worker pool and placement");
        return;
    }
    #[cfg(windows)]
    {
        win::no_throttling();
        let mut sets = win::cpu_sets();
        // (MCLA_CPUSET=test<n>: the first n processors taken as the fast class, to try the hybrid set-up on any machine)
        if let Some(k) = mode.strip_prefix("test").and_then(|v| v.parse::<usize>().ok()) {
            for (i, s) in sets.iter_mut().enumerate() {
                s.1 = u8::from(i < k);
            }
        }
        let mut classes: Vec<(u8, usize)> = Vec::new();
        for s in &sets {
            match classes.iter_mut().find(|c| c.0 == s.1) {
                Some(c) => c.1 += 1,
                None => classes.push((s.1, 1)),
            }
        }
        classes.sort();
        log::info!("cpu: {} logical processors, core classes (efficiency class, count) {:?}", sets.len(), classes);
        if classes.len() > 1 && mode != "nothrottle" {
            let top = classes.last().unwrap().0;
            let fast: Vec<u32> = sets.iter().filter(|s| s.1 == top).map(|s| s.0).collect();
            let n = fast.len().max(2);
            let ids = fast.clone();
            let built = rayon::ThreadPoolBuilder::new()
                .num_threads(n)
                .start_handler(move |_| win::keep_on(&ids))
                .build_global();
            win::keep_on(&fast);
            log::info!("cpu: hybrid - worker pool of {n} threads and the main thread on the {} processors of class {top}{}", fast.len(), if built.is_err() { " (the pool already existed: placement only)" } else { "" });
        }
    }
    #[cfg(not(windows))]
    log::info!("cpu: core classes not read on this system");
}

#[cfg(windows)]
mod win {
    use windows_sys::Win32::System::SystemInformation::GetSystemCpuSetInformation;
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcess, GetCurrentThread, ProcessPowerThrottling, SetProcessInformation, SetThreadSelectedCpuSets,
        PROCESS_POWER_THROTTLING_CURRENT_VERSION, PROCESS_POWER_THROTTLING_EXECUTION_SPEED, PROCESS_POWER_THROTTLING_STATE,
    };

    /// Execution speed throttling off for this process (control bit set, state bit clear: "never throttle").
    pub fn no_throttling() {
        let st = PROCESS_POWER_THROTTLING_STATE {
            Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
            ControlMask: PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
            StateMask: 0,
        };
        let ok = unsafe {
            SetProcessInformation(GetCurrentProcess(), ProcessPowerThrottling, &st as *const _ as *const core::ffi::c_void, std::mem::size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32)
        };
        if ok == 0 {
            log::info!("cpu: power throttling opt-out not taken (error {})", std::io::Error::last_os_error());
        }
    }

    /// (CPU set id, efficiency class) of every logical processor. The records are read as bytes (SYSTEM_CPU_SET_INFORMATION:
    /// Size u32 at 0, Type u32 at 4 - 0 for a CPU set -, Id u32 at 8, EfficiencyClass u8 at 18).
    pub fn cpu_sets() -> Vec<(u32, u8)> {
        let mut len = 0u32;
        unsafe { GetSystemCpuSetInformation(std::ptr::null_mut(), 0, &mut len, GetCurrentProcess(), 0) };
        if len == 0 {
            return Vec::new();
        }
        let mut buf = vec![0u64; (len as usize).div_ceil(8)];
        let ok = unsafe { GetSystemCpuSetInformation(buf.as_mut_ptr() as *mut _, len, &mut len, GetCurrentProcess(), 0) };
        if ok == 0 {
            return Vec::new();
        }
        let b = unsafe { std::slice::from_raw_parts(buf.as_ptr() as *const u8, len as usize) };
        let mut out = Vec::new();
        let mut o = 0usize;
        while o + 20 <= b.len() {
            let size = u32::from_le_bytes(b[o..o + 4].try_into().unwrap()) as usize;
            let ty = u32::from_le_bytes(b[o + 4..o + 8].try_into().unwrap());
            if size < 20 {
                break;
            }
            if ty == 0 {
                out.push((u32::from_le_bytes(b[o + 8..o + 12].try_into().unwrap()), b[o + 18]));
            }
            o += size;
        }
        out
    }

    /// The calling thread's selected CPU sets: the scheduler places it on these (a soft affinity, not a hard mask).
    pub fn keep_on(ids: &[u32]) {
        unsafe { SetThreadSelectedCpuSets(GetCurrentThread(), ids.as_ptr(), ids.len() as u32) };
    }
}
