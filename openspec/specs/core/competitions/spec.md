# core/competitions Specification

## Purpose
Assembles and simulates global international tournaments (e.g. a World Cup) by aggregating continental qualification results into a single worldwide competition and progressing it match by match.

## Requirements

### Requirement: Tournament assembly is date-gated to a single annual trigger
The system SHALL only attempt to assemble a global tournament on June 1st of the tournament's cycle year.

#### Scenario: Non-trigger date
- **WHEN** the simulated date is any day other than June 1
- **THEN** no tournament assembly is attempted for that day

### Requirement: Tournament year is derived from a fixed qualifying lead time
The system SHALL treat a competition configuration as due for assembly only when its qualifying cycle started exactly two years before the candidate tournament year.

#### Scenario: Configuration not due
- **WHEN** a competition's qualifying cycle did not start two years prior to the current year
- **THEN** that configuration is skipped for tournament assembly this cycle

### Requirement: Duplicate tournament assembly is prevented
The system SHALL NOT create a new tournament instance for a competition/year combination that already has an active tournament.

#### Scenario: Tournament already active
- **WHEN** a tournament for the same competition id and cycle year already exists
- **THEN** assembly for that configuration is skipped

### Requirement: Global tournament fields qualified teams from every continent
When assembling a tournament, the system SHALL collect qualified teams for the competition from all continents and combine them into a single qualified-teams list.

#### Scenario: No continent has qualified teams
- **WHEN** every continent reports zero qualified teams for a competition
- **THEN** the tournament is not assembled for that year

#### Scenario: Qualified teams exceed the tournament capacity
- **WHEN** the aggregated qualified teams exceed the competition's configured total team count
- **THEN** the qualified list is truncated to that total before the tournament is created

### Requirement: Newly assembled tournaments start in the group stage
A tournament created by assembly SHALL begin in the group stage phase with its groups drawn for the given year.

#### Scenario: Assembly completes
- **WHEN** a tournament is successfully assembled for a competition and year
- **THEN** its phase is set to group stage and its groups are drawn before any fixtures are played

### Requirement: Daily fixtures are resolved per active tournament phase
The system SHALL determine each active tournament's fixtures scheduled for a given date according to whether the tournament is in the group stage or knockout phase, and SHALL produce no fixtures for tournaments in any other phase.

#### Scenario: Group stage tournament on a matchday
- **WHEN** an active tournament is in the group stage and has fixtures scheduled for the current date
- **THEN** those group fixtures are included in today's matches

#### Scenario: Knockout stage tournament on a matchday
- **WHEN** an active tournament is in the knockout phase and has fixtures scheduled for the current date
- **THEN** those knockout fixtures are included in today's matches

### Requirement: Knockout draws are resolved via penalty shootout, with a deterministic fallback
For a knockout fixture level on aggregate score after regulation, the system SHALL take the winner from the engine's penalty shootout result when one was played; if no shootout was played, it SHALL fall back to a reputation-weighted random outcome.

#### Scenario: Shootout was played
- **WHEN** a knockout fixture ends level and the match engine recorded penalty shootout scores
- **THEN** the side with more shootout goals is recorded as the winner

#### Scenario: Shootout scores are also tied
- **WHEN** a knockout fixture's shootout scores are equal
- **THEN** the home side is recorded as the winner rather than the match being left unresolved

#### Scenario: No shootout was played on a level knockout fixture
- **WHEN** a knockout fixture ends level and no shootout result is present
- **THEN** the winner is chosen by a reputation-weighted random draw between the two sides, biased toward the higher-reputation country but bounded so no side's win chance falls outside a fixed range

### Requirement: Match results are recorded against the originating tournament and phase
Recording a fixture result SHALL update only the specific tournament, phase, group or bracket, and fixture slot that the fixture was drawn from.

#### Scenario: Group stage result recorded
- **WHEN** a group-stage fixture result is submitted
- **THEN** it is written to that tournament's matching group and fixture slot

#### Scenario: Knockout result recorded
- **WHEN** a knockout fixture result is submitted, including any penalty-shootout winner
- **THEN** it is written to that tournament's matching knockout bracket and fixture slot

### Requirement: Tournament phase transitions are checked once per simulated pass
After matches for the day are simulated, the system SHALL check every active tournament for phase completion, advancing group-stage tournaments to knockout once groups conclude and progressing knockout tournaments round by round.

#### Scenario: Groups complete
- **WHEN** all group fixtures for a tournament in the group stage have been played
- **THEN** the tournament's completion is evaluated and it may progress out of the group stage

#### Scenario: Knockout round complete
- **WHEN** a tournament in the knockout phase has results for its current round
- **THEN** the knockout bracket is progressed toward the next round
