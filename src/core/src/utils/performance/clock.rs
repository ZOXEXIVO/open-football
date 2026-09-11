//! Process-wide CPU accounting, without a dependency.
//!
//! Two counters, both cumulative since process start and both covering
//! every thread:
//!
//! * [`process_cpu_nanos`] — CPU time. Exact over long regions, but the OS
//!   only advances it on the scheduler tick, so it is useless below ~15 ms.
//! * [`process_cycles`] — CPU cycles. High resolution, which is what makes
//!   a millisecond-scale parallelism histogram possible, but expressed in
//!   cycles rather than seconds.
//!
//! The two together are self-calibrating: their ratio over the whole run is
//! this box's effective cycles per CPU second under this workload, so the
//! sampler needs no nominal clock rate and frequency scaling is already
//! baked in.

/// Total CPU time this process has burned across every thread, in
/// nanoseconds. Wall/CPU on either side of a region gives that region's
/// average busy-core count.
#[cfg(windows)]
#[inline]
pub fn process_cpu_nanos() -> u64 {
    use std::ffi::c_void;

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct FileTime {
        low: u32,
        high: u32,
    }

    unsafe extern "system" {
        fn GetCurrentProcess() -> *mut c_void;
        fn GetProcessTimes(
            process: *mut c_void,
            creation: *mut FileTime,
            exit: *mut FileTime,
            kernel: *mut FileTime,
            user: *mut FileTime,
        ) -> i32;
    }

    let mut creation = FileTime::default();
    let mut exit = FileTime::default();
    let mut kernel = FileTime::default();
    let mut user = FileTime::default();
    // SAFETY: the pseudo-handle from `GetCurrentProcess` needs no close, and
    // all four out-params are live, correctly sized `FILETIME`s.
    let ok = unsafe {
        GetProcessTimes(
            GetCurrentProcess(),
            &mut creation,
            &mut exit,
            &mut kernel,
            &mut user,
        )
    };
    if ok == 0 {
        return 0;
    }
    // FILETIME counts 100 ns units.
    let nanos = |f: FileTime| (((f.high as u64) << 32) | f.low as u64) * 100;
    nanos(kernel) + nanos(user)
}

/// Linux flavour: `utime + stime` out of `/proc/self/stat`, converted with
/// the conventional 100 Hz `USER_HZ`. Coarser than the Windows call, but the
/// phases here are tens of milliseconds and up, and it keeps the profiler
/// dependency-free.
#[cfg(not(windows))]
#[inline]
pub fn process_cpu_nanos() -> u64 {
    let Ok(stat) = std::fs::read_to_string("/proc/self/stat") else {
        return 0;
    };
    // Field 2 is the comm, which may itself contain spaces — split after its
    // closing paren so the field indices below are stable.
    let Some(rest) = stat.rsplit_once(')').map(|(_, r)| r) else {
        return 0;
    };
    let fields: Vec<&str> = rest.split_whitespace().collect();
    // After the comm, index 0 is `state`; utime/stime are fields 14/15 of the
    // whole line, i.e. 11/12 from here.
    let ticks = |i: usize| {
        fields
            .get(i)
            .and_then(|f| f.parse::<u64>().ok())
            .unwrap_or(0)
    };
    (ticks(11) + ticks(12)) * 10_000_000
}

/// CPU cycles burned by every thread in this process, cumulative since
/// process start. Unlike [`process_cpu_nanos`] this is not quantised to the
/// scheduler tick, so it can resolve a straggler tail inside a single tick.
#[cfg(windows)]
#[inline]
pub fn process_cycles() -> u64 {
    use std::ffi::c_void;

    unsafe extern "system" {
        fn GetCurrentProcess() -> *mut c_void;
        fn QueryProcessCycleTime(process: *mut c_void, cycle_time: *mut u64) -> i32;
    }

    let mut cycles: u64 = 0;
    // SAFETY: the pseudo-handle needs no close; `cycles` is a live u64
    // out-param.
    let ok = unsafe { QueryProcessCycleTime(GetCurrentProcess(), &mut cycles) };
    if ok == 0 { 0 } else { cycles }
}

/// Elsewhere there is no cheap process-wide cycle counter, so the histogram
/// falls back to the same coarse CPU clock the phase table uses. Its buckets
/// are then only meaningful at sampling periods well above the platform's
/// CPU-accounting granularity.
#[cfg(not(windows))]
#[inline]
pub fn process_cycles() -> u64 {
    process_cpu_nanos()
}
