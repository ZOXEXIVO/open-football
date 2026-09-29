# core/club/player/generators Specification

## Purpose
Defines how the core player generator builds academy intakes: attribute profiles shaped by exact position, role
archetype and age, and held within the sampled potential and the age caps.

## Requirements

### Requirement: Generated academy players never exceed their sampled potential

An academy player's generated profile SHALL score no higher than his sampled PA for his position. When floors or talent
spikes push the profile above his PA, it SHALL be fitted down to that PA. The sampled PA SHALL NOT be raised to cover
the generated CA.

#### Scenario: A low-PA intake
- **WHEN** academy skills are generated for PA 20..200 at ages 14, 17 and 19, for every position group
- **THEN** each profile's scored CA is at or below its PA

### Requirement: Generated academy attributes respect the age cap

Every generated attribute SHALL lie between 1 and the age's skill cap. For a goalkeeper this includes his goalkeeping
attributes. Match readiness SHALL be excluded from attribute noise and SHALL start between 10 and 15.

#### Scenario: A 14-year-old keeper
- **WHEN** a 14-year-old goalkeeper is generated at a high PA
- **THEN** none of his outfield or goalkeeping attributes exceed the age-14 cap, and his match readiness is between 10
  and 15

### Requirement: Academy profiles follow the exact position and one archetype

Academy generation SHALL shape attributes with the exact position's generation weights, not a four-bucket group.
It SHALL draw one role-archetype roll per player and use it for both his outfield shape and, for a goalkeeper, his
keeper profile. Per-family maturity SHALL be read at the middle of the player's age year.

#### Scenario: A keeper's two profiles agree
- **WHEN** a goalkeeper is generated
- **THEN** his outfield archetype and his goalkeeping archetype come from the same roll
