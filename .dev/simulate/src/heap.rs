//! mimalloc behind a counter, lent to the profiler as its allocation probe.
//!
//! Counting only starts when `OF_PERFORMANCE` is set, so a timing run pays a
//! single relaxed load per allocation. Process totals live in cache-padded
//! shards, one per thread modulo the shard count, so 60-odd threads never
//! fight over one counter; each thread also keeps its own running total,
//! which is what a profiler stage reads.

use core::{AllocationCount, AllocationProbe, PerformanceProfiler};
use mimalloc::MiMalloc;
use std::alloc::{GlobalAlloc, Layout};
use std::cell::Cell;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

const SHARD_COUNT: usize = 64;

#[repr(align(128))]
struct Shard {
    allocations: AtomicU64,
    reallocations: AtomicU64,
    bytes: AtomicU64,
}

impl Shard {
    const fn new() -> Self {
        Shard {
            allocations: AtomicU64::new(0),
            reallocations: AtomicU64::new(0),
            bytes: AtomicU64::new(0),
        }
    }
}

static COUNTING: AtomicBool = AtomicBool::new(false);
static SHARDS: [Shard; SHARD_COUNT] = [const { Shard::new() }; SHARD_COUNT];
static NEXT_SHARD: AtomicUsize = AtomicUsize::new(0);

thread_local! {
    static THREAD_SHARD: Cell<usize> = const { Cell::new(usize::MAX) };
    static THREAD_COUNT: Cell<AllocationCount> = const {
        Cell::new(AllocationCount {
            allocations: 0,
            reallocations: 0,
            bytes: 0,
        })
    };
}

pub struct CountingAllocator;

impl CountingAllocator {
    /// Start counting and lend the counters to the profiler — only when the
    /// profiler is going to read them.
    pub fn install_probe_if_profiling() {
        if std::env::var_os("OF_PERFORMANCE").is_none() {
            return;
        }
        COUNTING.store(true, Ordering::Relaxed);
        PerformanceProfiler::install_allocation_probe(AllocationProbe {
            process: Self::process_count,
            thread: Self::thread_count,
        });
    }

    pub fn is_counting() -> bool {
        COUNTING.load(Ordering::Relaxed)
    }

    /// Everything allocated on every thread since counting started. `bytes`
    /// is what allocations asked for plus what reallocations grew by.
    pub fn process_count() -> AllocationCount {
        SHARDS.iter().fold(AllocationCount::default(), |sum, s| AllocationCount {
            allocations: sum.allocations + s.allocations.load(Ordering::Relaxed),
            reallocations: sum.reallocations + s.reallocations.load(Ordering::Relaxed),
            bytes: sum.bytes + s.bytes.load(Ordering::Relaxed),
        })
    }

    /// Everything the calling thread has allocated since counting started.
    pub fn thread_count() -> AllocationCount {
        THREAD_COUNT.try_with(Cell::get).unwrap_or_default()
    }

    #[inline(always)]
    fn count(bytes: usize, reallocation: bool) {
        if !COUNTING.load(Ordering::Relaxed) {
            return;
        }
        let shard = &SHARDS[Self::shard_index()];
        if reallocation {
            shard.reallocations.fetch_add(1, Ordering::Relaxed);
        } else {
            shard.allocations.fetch_add(1, Ordering::Relaxed);
        }
        shard.bytes.fetch_add(bytes as u64, Ordering::Relaxed);
        let _ = THREAD_COUNT.try_with(|cell| {
            let mut own = cell.get();
            if reallocation {
                own.reallocations += 1;
            } else {
                own.allocations += 1;
            }
            own.bytes += bytes as u64;
            cell.set(own);
        });
    }

    #[inline(always)]
    fn shard_index() -> usize {
        THREAD_SHARD
            .try_with(|cell| {
                let ix = cell.get();
                if ix != usize::MAX {
                    return ix;
                }
                let ix = NEXT_SHARD.fetch_add(1, Ordering::Relaxed) % SHARD_COUNT;
                cell.set(ix);
                ix
            })
            .unwrap_or(0)
    }
}

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        Self::count(layout.size(), false);
        unsafe { MiMalloc.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        Self::count(layout.size(), false);
        unsafe { MiMalloc.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { MiMalloc.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        Self::count(new_size.saturating_sub(layout.size()), true);
        unsafe { MiMalloc.realloc(ptr, layout, new_size) }
    }
}
