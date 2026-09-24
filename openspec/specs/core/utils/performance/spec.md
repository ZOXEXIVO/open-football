# core/utils/performance Specification

## Purpose
Provides an opt-in wall/CPU profiler for the daily world-simulation tick, measuring phase and stage timings and the busy-core width of each profiled region with zero overhead when disabled.

## Requirements

### Requirement: Opt-in wall/CPU performance profiling for the simulation tick
The system SHALL provide an opt-in profiler, enabled only via an environment flag, that measures both wall time and process CPU time for named regions of the daily simulation tick, and SHALL leave zero measurable overhead when disabled.

#### Scenario: Profiling is disabled
- **WHEN** the enabling environment variable is not set
- **THEN** profiling calls execute the wrapped work directly without recording any timing data or starting a background sampling thread

#### Scenario: Profiling is enabled for a driving-thread region
- **WHEN** a region on the tick's driving thread is profiled as a "phase"
- **THEN** its accumulated wall time and process CPU time are recorded, and the ratio of CPU time to wall time reflects the average number of cores busy during that phase

#### Scenario: Profiling is enabled for a parallel region
- **WHEN** a region running inside a parallel worker pool is profiled as a "stage"
- **THEN** the summed time across every worker and the single slowest item are recorded, so the ratio of the sum to the maximum indicates the widest that stage could ever run

### Requirement: Per-phase busy-core width reporting
When profiling is enabled, the system SHALL sample the process's CPU cycle consumption at a fixed interval and SHALL report, per profiled phase, how much of its wall time ran at each range of busy-core counts, distinguishing a phase that is uniformly wide from one with a long single-core straggler tail.

#### Scenario: A phase runs at consistently high parallelism
- **WHEN** a phase's sampled windows show cycle consumption near the host's full core count throughout its duration
- **THEN** nearly all of its wall time is attributed to the highest core-count bucket

#### Scenario: A phase has a straggler tail
- **WHEN** a phase runs at high parallelism for part of its duration and at very low parallelism for the remainder
- **THEN** the reported wall time is split across both a high-core bucket and a low-core bucket, revealing the tail that an average core count alone would hide
