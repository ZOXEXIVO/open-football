# core/utils/random Specification

## Purpose
Provides the simulation's seedable random-number source and the uniform integer, float, and alphabetic-string draws every other module builds randomness on top of.

## Requirements

### Requirement: Reproducible seeded randomness
The system SHALL provide a process-global random number source whose seed can be pinned so simulation runs are reproducible, while defaulting to an unpinned, process-unique stream when no seed is set.

#### Scenario: Seed is pinned before a run
- **WHEN** a caller sets an explicit seed before building the simulation world
- **THEN** every thread's random draws are derived from that seed combined with a per-thread identifier, so the same seed produces the same per-thread draw sequence on a later run

#### Scenario: No seed has been set
- **WHEN** no explicit seed has been pinned
- **THEN** each thread seeds itself from a process-unique base value mixed with its thread identifier, so distinct threads do not share a random stream

### Requirement: Uniform random values in a caller-specified range
The system SHALL provide uniform random integer and floating-point generation bounded by a caller-supplied minimum and maximum.

#### Scenario: Requesting a random integer
- **WHEN** a caller asks for a random integer between a minimum and a maximum
- **THEN** the returned value is derived from a uniform draw scaled into that range

#### Scenario: Requesting a random float
- **WHEN** a caller asks for a random float between a minimum and a maximum
- **THEN** the returned value is derived from a uniform draw scaled into that range

### Requirement: Random alphabetic string generation
The system SHALL be able to generate a random alphabetic string of a requested length, with the first character uppercase and the remaining characters lowercase.

#### Scenario: Generating an identifier-like string
- **WHEN** a caller requests a random string of length N
- **THEN** the result has exactly N characters, the first is an uppercase letter, and the rest are lowercase letters
