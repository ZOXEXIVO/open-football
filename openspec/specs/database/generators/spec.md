# database/generators Specification

## Purpose
Defines the procedural generation behavior that fills in players, staff, clubs, leagues, and countries wherever the shipped seed data is absent, thin, or does not cover a given squad slot.

## Requirements

### Requirement: A fresh world always opens at a fixed, reproducible calendar point
The generated world's opening date SHALL be 1 August of the real-world year the world is generated in, computed once per process.

#### Scenario: The opening date matches the season it claims to start
- **WHEN** a new world is generated
- **THEN** its start date SHALL be 1 August, and the season computed from that date SHALL report the same year as its start year

### Requirement: Procedurally generated players never collide in id with seed-supplied players
The procedural id sequence used for generated players SHALL be seeded past the highest id present in the external seed data before any generation begins, and re-seeded past the highest id present anywhere in the fully assembled world after generation completes.

#### Scenario: A generated filler player never reuses a seed-database id
- **WHEN** a club has no seed-supplied players for a given team and the generator fills that team procedurally
- **THEN** none of the generated players' ids SHALL collide with any id present in the seed player database

### Requirement: A club with any seed-supplied players is fully seed-backed at the senior level
When a club has at least one seed-supplied player record, every senior team type (Main, Reserve, B, U20, U21, U23) SHALL be populated exclusively from seed records for that club; senior squads are never a mix of seed-supplied and procedurally generated players. Academy teams (U18/U19) are always populated by the academy generator regardless of seed-data presence.

#### Scenario: A senior team bucket with no seed records for that type is left empty rather than backfilled
- **WHEN** a club has seed player records but none of them place into a particular senior team type
- **THEN** that team type SHALL end up with zero players rather than being topped up procedurally

### Requirement: Seed-supplied players are placed into a squad primarily by age, with a senior-readiness override
A seed-supplied player SHALL be placed into a squad bucket according to an age ladder (U18 at or below 18, U19 at 19, U20 at 20, U21 at 21, U23 at 23, otherwise the senior squad), EXCEPT that a player at least 17 years old whose current ability meets or exceeds his club's own senior-quality floor for his position SHALL be placed directly into the senior squad regardless of his age.

#### Scenario: An age-ladder placement is overridden for an ability-proven teenager
- **WHEN** a 19-year-old seed-supplied player's current ability is at or above the level of his club's established (21+) players at his position, given the club's reputation
- **THEN** he SHALL be placed in the club's senior squad rather than an age-appropriate youth squad

#### Scenario: The age floor holds regardless of ability below the minimum senior-ready age
- **WHEN** a seed-supplied player is younger than 17
- **THEN** he SHALL be placed by the age ladder alone, never promoted to the senior squad by ability

#### Scenario: An explicit placement hint from the data always wins
- **WHEN** a seed-supplied player record carries an explicit team-type placement hint and the club has a matching squad bucket
- **THEN** he SHALL be placed in that bucket, overriding both the age ladder and the senior-readiness override

### Requirement: Procedurally generated squads meet minimum size and position-coverage floors
For a team without seed data, the generator SHALL produce a squad covering all four position buckets (goalkeeper, defender, midfielder, striker) in every generated team, and SHALL guarantee at least 25 total players for a Main (first) team.

#### Scenario: A generated Main squad never falls below 25 players
- **WHEN** a Main team is generated procedurally and the randomised per-bucket counts sum below 25
- **THEN** additional outfield players SHALL be generated until the squad reaches 25

### Requirement: Procedurally generated player roles are distributed across the squad rather than clustered
Each generated player SHALL be assigned a squad role (e.g. Star, Starter, Rotation, Backup, Prospect, Fringe) drawn from a per-position, per-team-type quota so that top-tier roles cannot all land on one position group by chance.

#### Scenario: A Main squad's top roles are not all goalkeepers
- **WHEN** a Main team is generated procedurally
- **THEN** the Star/Starter role allocations SHALL be spread across defender/midfielder/striker buckets according to the team-type's role quota, not concentrated arbitrarily on goalkeepers

### Requirement: A generated player's current and potential ability derive continuously from context
A generated player's current ability SHALL be derived from a blend of team, league, and country reputation, modulated continuously by the player's assigned squad role and age (no hard reputation-tier branching); potential ability SHALL equal current ability plus a role-and-age-aware headroom, with prospects receiving materially more headroom than established stars.

#### Scenario: A young prospect carries a larger ability ceiling than an established veteran of equal current ability
- **WHEN** two generated players share the same current ability but one is assigned the Prospect role at a young age and the other the Starter role at an older age
- **THEN** the prospect's potential ability SHALL exceed the veteran's by a materially larger margin

### Requirement: Procedural generation is not reproducible; seed-record hydration is
Procedurally generated players SHALL draw from an entropy-seeded random stream (not reproducible across runs). Players hydrated from an external seed record SHALL draw from a stream seeded deterministically by that record's id, so the same seed record always hydrates to the same attributes.

#### Scenario: Re-hydrating the same seed record twice yields identical attributes
- **WHEN** the same external player record is hydrated into a player twice (e.g. across two fresh saves)
- **THEN** the resulting skill values, position, and body metrics SHALL be identical both times

#### Scenario: Two procedurally generated players from the same call are not forced identical
- **WHEN** two players are generated procedurally in the same process
- **THEN** their generated attributes SHALL NOT be guaranteed identical, and re-running generation SHALL NOT reproduce the same values

### Requirement: Foreign-player composition of a generated squad follows the best available authority
When filling a team procedurally, the generator SHALL prefer a league-specific foreign-player composition list when the league defines one; otherwise it SHALL derive a composition from the player's country's shipped transfer-market card (import corridors and foreign-player share); a country with neither SHALL fill entirely domestically.

#### Scenario: A country's shipped foreign-player share is honored when no league override exists
- **WHEN** a league has no foreign-player list of its own but its country ships a transfer card naming import corridors and a foreign-player share
- **THEN** the generated squad's expected proportion of foreign-sourced fillers SHALL equal the country's shipped foreign-player share, distributed across corridors in proportion to their weights

### Requirement: Nationality and region influence a generated player's skill profile
A generated player's skills SHALL receive an additive adjustment based on his nationality (or, absent a specific nationality rule, his continental region), reflecting recognizable but non-deterministic national playing-style tendencies. This adjustment MUST NOT make players from the same country identical to one another.

#### Scenario: Players from a country with an authored bias trend toward its profile without collapsing into uniformity
- **WHEN** many players are generated for a country that has an authored skill bias
- **THEN** the biased skills SHALL trend higher (or lower) on average than an otherwise-identical unbiased population, while individual players SHALL still vary from each other

### Requirement: A generated Main team always has exactly one manager and a viable backroom
A newly generated Main team SHALL be created with exactly one permanent manager and a non-empty operational backroom staff, scaled by team reputation; a generated non-Main team SHALL never receive its own manager seat.

#### Scenario: Even the smallest generated club fields a manager and support staff
- **WHEN** a Main team is generated for a club of minimal reputation
- **THEN** the generated staff SHALL include exactly one manager and at least a minimal operational backroom (never zero staff)

#### Scenario: A youth or reserve team never gets a second manager
- **WHEN** a non-Main team (e.g. U18) is generated
- **THEN** no manager or caretaker-manager role SHALL be generated for that team

### Requirement: Every generated Main team fields a goalkeeping coach
A generated Main team's backroom staff SHALL always include a goalkeeper coach, regardless of club reputation.

#### Scenario: A minimal-reputation club still has a goalkeeping coach
- **WHEN** a Main team is generated for a club at the lowest reputation tier
- **THEN** the generated staff SHALL still include exactly one goalkeeper coach

### Requirement: Scout market knowledge is seeded from the club's country's own trade patterns
A generated scout SHALL start with full knowledge of his employing club's home country and, when the country ships a transfer-market card, partial knowledge of a reputation-scaled number of foreign markets sampled from that country's import corridors (rather than generic regional knowledge). Countries without a shipped card fall back to a region-level corridor table.

#### Scenario: Scouts at clubs in different countries start knowing different foreign markets
- **WHEN** two clubs in different countries with different shipped import corridors each generate a scout
- **THEN** the two scouts' seeded foreign market knowledge SHALL reflect their respective countries' own import corridors, not a shared generic list
