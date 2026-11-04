# core/utils/random Specification

## Purpose
Provides the simulation's seedable random-number source and the uniform integer, float, and alphabetic-string draws every other module builds randomness on top of.

## Requirements

### Requirement: Reproducible seeded randomness
The system SHALL provide a process-global random number source whose seed can be pinned so simulation runs are reproducible, while defaulting to an unpinned, process-unique stream when no seed is set.

While a seed is pinned:

- Every random draw made while generating the world, ticking it and playing its matches SHALL derive from that seed, and none SHALL come from an operating-system entropy source.
- Each match SHALL draw from a stream derived from the seed and the fixture's identity: its date, its competition, and its home and away sides. A fixture therefore plays out the same whichever worker or machine plays it and wherever it sits in its batch.
- Two runs from the same seed and the same database, with one worker in each thread pool, SHALL produce identical worlds after every simulated day.

#### Scenario: Seed is pinned before a run
- **WHEN** a caller sets an explicit seed before building the simulation world
- **THEN** every thread's random draws are derived from that seed combined with a per-thread identifier, so the same seed produces the same per-thread draw sequence on a later run

#### Scenario: No seed has been set
- **WHEN** no explicit seed has been pinned
- **THEN** each thread seeds itself from a process-unique base value mixed with its thread identifier, so distinct threads do not share a random stream
- **AND** world generation and every match keep drawing fresh entropy

#### Scenario: Two single-worker runs from the same seed
- **WHEN** the same database is generated and ticked for 30 days twice with the same pinned seed and one worker in each thread pool
- **THEN** after every day both runs report identical match results and identical player, club, staff, free-agent and transfer-market state

#### Scenario: A fixture's result does not depend on its batch
- **WHEN** a seed is pinned and the same fixture, with the same two squads, is played as part of two batches that order their matches differently
- **THEN** both plays produce the same result

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
