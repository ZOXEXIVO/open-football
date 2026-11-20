//! The machine a census has to fit on: free physical memory, read so a run
//! stops and reports before the machine starts paging, and the census's own
//! resident memory.

pub struct MemoryFloor;

impl MemoryFloor {
    /// Below this much free memory the census stops and reports.
    pub const FLOOR_GB: f64 = 2.5;

    pub fn reached() -> bool {
        Self::free_gigabytes() < Self::FLOOR_GB
    }
}

#[cfg(windows)]
impl MemoryFloor {
    pub fn free_gigabytes() -> f64 {
        #[repr(C)]
        #[derive(Default)]
        struct MemoryStatusEx {
            length: u32,
            memory_load: u32,
            total_phys: u64,
            avail_phys: u64,
            total_page: u64,
            avail_page: u64,
            total_virtual: u64,
            avail_virtual: u64,
            avail_ext: u64,
        }
        unsafe extern "system" {
            fn GlobalMemoryStatusEx(buffer: *mut MemoryStatusEx) -> i32;
        }
        let mut status = MemoryStatusEx {
            length: size_of::<MemoryStatusEx>() as u32,
            ..Default::default()
        };
        if unsafe { GlobalMemoryStatusEx(&mut status) } == 0 {
            return f64::INFINITY;
        }
        status.avail_phys as f64 / (1024.0 * 1024.0 * 1024.0)
    }

    pub fn resident_gigabytes() -> f64 {
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
        let ok =
            unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, counters.size) };
        if ok == 0 {
            return 0.0;
        }
        counters.peak_working_set as f64 / (1024.0 * 1024.0 * 1024.0)
    }
}

#[cfg(not(windows))]
impl MemoryFloor {
    pub fn free_gigabytes() -> f64 {
        std::fs::read_to_string("/proc/meminfo")
            .ok()
            .and_then(|s| {
                s.lines()
                    .find(|l| l.starts_with("MemAvailable:"))?
                    .split_whitespace()
                    .nth(1)?
                    .parse::<f64>()
                    .ok()
            })
            .map_or(f64::INFINITY, |kb| kb / (1024.0 * 1024.0))
    }

    pub fn resident_gigabytes() -> f64 {
        std::fs::read_to_string("/proc/self/status")
            .ok()
            .and_then(|s| {
                s.lines()
                    .find(|l| l.starts_with("VmHWM:"))?
                    .split_whitespace()
                    .nth(1)?
                    .parse::<f64>()
                    .ok()
            })
            .map_or(0.0, |kb| kb / (1024.0 * 1024.0))
    }
}
