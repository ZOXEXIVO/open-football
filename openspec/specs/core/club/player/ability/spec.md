# core/club/player/ability Specification

## Purpose
Defines how a player's visible current ability (CA) is scored from his attributes, the shared inverse that turns an
ability budget into a skill level, and how an attribute profile is fitted to a target CA without losing its shape.

## Requirements

### Requirement: Visible ability is scored on a calibrated, position-specific scale

A player's visible CA for an exact position SHALL be computed in three steps:

- take a weighted average of all his attributes, using weights specific to that position
- map that average as `round(19 × average − 110)`
- clamp the result to 1..200

Scoring weights SHALL be non-negative and SHALL sum to one. Match readiness SHALL carry zero weight. Scoring weights
SHALL be distinct from the weights that shape generated profiles. Goalkeepers SHALL be scored through the same
mechanism, with their own position table.

#### Scenario: A flat profile round-trips for every position
- **WHEN** a profile with every attribute at the skill level for CA `n` is scored for any exact position, for any `n`
  in 1..200
- **THEN** the scored CA is exactly `n`

#### Scenario: Held-out source profiles score close to their recorded CA
- **WHEN** the checked-in held-out calibration profiles (player id divisible by five) are scored for their recorded
  positions
- **THEN** the mean absolute error against their recorded CA is below 5 points

#### Scenario: Match readiness has no ability cost
- **WHEN** only a player's match readiness changes
- **THEN** his scored CA does not change

### Requirement: One skill-level inverse serves every consumer of the CA scale

The flat skill level corresponding to an ability `a` SHALL be `(clamp(a, 1, 200) + 110) / 19`. Player generation,
academy ceilings, development ceilings and synthetic test players SHALL all size profiles with this inverse, and none
of them SHALL use a separate PA-to-skill formula.

#### Scenario: A synthetic player of a known level
- **WHEN** a flat profile is built for a target visible ability
- **THEN** every attribute equals the inverse of that target, and the profile scores that target

### Requirement: A role-shaped profile can be sized to an ability

Given a position, an ability and a role shape (a per-attribute multiplier), the system SHALL return the base level `L`
at which the profile `L × shape` scores that ability for the position.

#### Scenario: A role whose shape under-weights its costly attributes
- **WHEN** the base level is computed for a fullback shape and for a striker shape at the same ability
- **THEN** each shape, multiplied by its own base level, scores that ability for its own position

### Requirement: Fitting to a target CA preserves the profile's shape

Fitting a profile to a target CA SHALL scale its attributes by one common factor, chosen by searching the actual
scoring function, until the profile scores the target. Relative strengths SHALL be kept until an attribute reaches 1
or the supplied cap. Absent (zero) attributes SHALL stay absent, and match readiness SHALL never be scaled. When a cap
makes the target unreachable, fitting SHALL return the closest CA it can reach.

#### Scenario: Every CA is reachable for outfielders and keepers
- **WHEN** an uneven profile is fitted to each target from 1 to 200, as a striker and as a goalkeeper, with a cap of 20
- **THEN** each fitted profile scores exactly its target, and its match readiness is unchanged

#### Scenario: Extreme targets do not flatten the profile
- **WHEN** a striker profile with high finishing and low tackling is fitted to CA 1 or to CA 200
- **THEN** finishing still clearly exceeds tackling after fitting

#### Scenario: An outfielder's goalkeeping stays absent
- **WHEN** an outfielder whose goalkeeping attributes are zero is fitted to a higher CA
- **THEN** his goalkeeping attributes remain zero

#### Scenario: A cap bounds the reachable ability
- **WHEN** a keeper profile is fitted to CA 200 under a cap of 12
- **THEN** no attribute exceeds 12, and the returned CA is the highest the cap allows

### Requirement: Fitting beside recorded values moves only generated slots, within a band

When fitting a profile beside a set of recorded values, the recorded values SHALL stay fixed. Generated slots SHALL
flex by a factor between 0.8 and 1.25 of their starting values, and no further. Any gap still left after that SHALL
remain with the record.

#### Scenario: The record's own gap stays with the record
- **WHEN** a profile is fitted beside a recorded profile whose CA is far from the target
- **THEN** the generated slots stop at the edge of the flex band, the recorded values are unchanged, and the returned
  CA differs from the target
