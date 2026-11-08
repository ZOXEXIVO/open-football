//! Resident memory, printed at each report so a whole-career run can be held
//! to the machine it has to fit on.

pub struct ResidentMemory;

#[cfg(windows)]
impl ResidentMemory {
    pub fn gigabytes() -> f64 {
        #[repr(C)]
        #[derive(Default)]
        struct ProcessMemoryCounters {
            size: u32,
            page_fault_count: u32,
            peak_working_set: usize,
            working_set: usize,
            quota_peak_paged_pool: usize,
            quota_paged_pool: usize,
            quota_peak_nonpaged_pool: usize,
            quota_nonpaged_pool: usize,
            pagefile: usize,
            peak_pagefile: usize,
        }
        unsafe extern "system" {
            fn GetCurrentProcess() -> *mut std::ffi::c_void;
            fn K32GetProcessMemoryInfo(
                process: *mut std::ffi::c_void,
                counters: *mut ProcessMemoryCounters,
                size: u32,
            ) -> i32;
        }
        let mut counters = ProcessMemoryCounters {
            size: size_of::<ProcessMemoryCounters>() as u32,
            ..Default::default()
        };
        let ok = unsafe {
            K32GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, counters.size)
        };
        if ok == 0 {
            return 0.0;
        }
        counters.working_set as f64 / (1024.0 * 1024.0 * 1024.0)
    }
}

#[cfg(not(windows))]
impl ResidentMemory {
    pub fn gigabytes() -> f64 {
        std::fs::read_to_string("/proc/self/statm")
            .ok()
            .and_then(|s| s.split_whitespace().nth(1)?.parse::<f64>().ok())
            .map_or(0.0, |pages| pages * 4096.0 / (1024.0 * 1024.0 * 1024.0))
    }
}
