# core/utils Specification

## Purpose
Provides cross-cutting utilities shared by the whole simulation — CPU feature detection, calendar helpers, time estimation, money/fee formatting, logging with slow-call thresholds, seedable randomness, and an opt-in performance profiler for the daily world-simulation tick.

## Requirements

### Requirement: Calendar predicates for simulation cadence
The system SHALL expose date predicates used to drive periodic simulation events: birthday matching, age-in-years, quarter start, year start, year end, month beginning, and the next Saturday on or after a given date.

#### Scenario: Checking a birthday
- **WHEN** a player's birth date and the current simulation date share the same month and day
- **THEN** the date is reported as the player's birthday, regardless of the year

#### Scenario: Computing age
- **WHEN** age is computed from a birth date and a current date
- **THEN** the result is the whole number of 365-day years elapsed between the two dates

#### Scenario: Finding the next Saturday
- **WHEN** the next Saturday is requested for a given date
- **THEN** the result is that same date if it is already a Saturday, otherwise the nearest following Saturday

### Requirement: Fee and money formatting for negotiation and display
The system SHALL round a monetary amount to a "nice" negotiation-friendly value and SHALL format monetary amounts into a compact human-readable string with K/M suffixes.

#### Scenario: Rounding a fee for negotiation
- **WHEN** a fee amount is rounded via the fee-rounding rule
- **THEN** amounts under 1,000 round to the nearest 100, amounts from 1,000 up to 100,000 round to the nearest 1,000, amounts from 100,000 up to 1,000,000 round to the nearest 10,000, and amounts of 1,000,000 or more round to the nearest 100,000

#### Scenario: Formatting a large amount for display
- **WHEN** an amount of at least 1,000,000 is formatted for display
- **THEN** it is rendered as a value in millions with one decimal place and an "M" suffix

#### Scenario: Formatting a mid-size amount for display
- **WHEN** an amount of at least 1,000 but under 1,000,000 is formatted for display
- **THEN** it is rendered as a value in thousands with one decimal place and a "K" suffix

### Requirement: CPU feature detection for SIMD dispatch
The system SHALL detect at runtime whether the host CPU supports AVX2 (x86_64) or NEON (AArch64) and SHALL expose a single human-readable name for the SIMD code path that will be used.

#### Scenario: Selecting a SIMD kernel on x86_64 with AVX2
- **WHEN** the running CPU advertises AVX2 support
- **THEN** the reported kernel name is "AVX2"

#### Scenario: Selecting a SIMD kernel with no supported extension
- **WHEN** the running CPU advertises neither AVX2 nor NEON
- **THEN** the reported kernel name is "scalar"

### Requirement: Timed execution with slow-call logging
The system SHALL measure the wall-clock duration of an arbitrary action and SHALL emit a log message at elevated severity only when that duration exceeds a fixed threshold, avoiding log noise for fast calls.

#### Scenario: Action completes quickly
- **WHEN** a timed action completes in under the configured duration threshold
- **THEN** no informational log entry is emitted for it (only a debug-level trace, when applicable)

#### Scenario: Action exceeds the threshold
- **WHEN** a timed action's duration exceeds the configured threshold (1000 ms)
- **THEN** an informational log entry reporting the elapsed time is emitted

## Child specs

The requirements above live directly under `utils/` with no subfolder of their own. The following requirements moved into their own capability specs, one per subfolder:

- [performance](performance/spec.md)
- [random](random/spec.md)
