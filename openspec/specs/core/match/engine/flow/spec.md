# core/match/engine/flow Specification

## Purpose
Owns the match clock and the produced match result — the scoreline, its derived outcome, highlights, and each player's end-of-involvement physical snapshot.

## Requirements

### Requirement: Match clock and stoppage time
The match clock SHALL advance in fixed increments for the duration of each timed period, and SHALL accumulate additional (stoppage) time within a period up to a bounded cap rather than extending a period indefinitely. The stoppage added to a period SHALL be derived from the dead-ball time actually measured in that period — restarts awaited, goal celebrations, substitutions, injury treatment and deliberate delay — passed through the referee's profile, rather than from fixed per-event constants, so that a period with more stoppages ends later and a period with none ends on time.

#### Scenario: Stoppage accumulates during a period
- **WHEN** the engine measures dead-ball time during the first or second half or an extra-time period
- **THEN** that period's effective end time is extended by the referee's allowance for that dead time, up to the period's stoppage cap

#### Scenario: Stoppage not recorded outside a timed period
- **WHEN** dead-ball time is measured while the match is in an interval, a shootout, or has ended
- **THEN** it has no effect on any period's length

#### Scenario: Busier period gets more stoppage
- **WHEN** two otherwise-identical periods differ only in that one contains two substitutions and an injury treatment
- **THEN** the busier period's stoppage time is strictly greater than the other's

### Requirement: Match result recording
On completion, the system SHALL produce a match result carrying each team's final score (including any penalty shootout tally), the goals that were scored with enough detail to reconstruct each one, and an outcome (home win / away win / draw) derived from the regulation-plus-extra-time score, falling back to the shootout tally when that score is level.

#### Scenario: Regulation score decides the outcome
- **WHEN** a match ends with the two teams' goal tallies unequal after any extra time
- **THEN** the recorded outcome favors the team with the higher tally, regardless of any shootout data present

#### Scenario: Shootout breaks a level tie
- **WHEN** a match's regulation-plus-extra-time score is level and a penalty shootout took place
- **THEN** the recorded outcome favors whichever side won the shootout, and a level shootout is never itself recorded as the deciding state

### Requirement: Player physical snapshot at match exit
For every player who featured in the match, the system SHALL record a physical snapshot of the player's condition at the moment they left the pitch (substituted or full time), computed from their entry time to their exit time.

#### Scenario: Substituted player's stats freeze at the substitution
- **WHEN** a player is replaced before full time
- **THEN** that player's recorded minutes played and physical snapshot reflect the moment of substitution rather than the remainder of the match

### Requirement: The match result carries the standard of football
Every completed match SHALL record, in its result, the standard of football the match read from both sides at kickoff.
The value SHALL be the same one the match used for the whole of its play. A result produced by a remote match worker
SHALL carry the same value as a result produced locally for the same fixture, squads and seed.

#### Scenario: A completed match reports its standard
- **WHEN** a competitive match completes
- **THEN** its result includes the standard of football read at kickoff

#### Scenario: Local and remote results agree
- **WHEN** the same fixture, squads and seed are played once locally and once on a remote match worker
- **THEN** both results report the same standard of football

### Requirement: The match result records missed penalties
Every completed match SHALL record, in its result, each in-play penalty that was not scored and who took it. This sits
beside the record of which keeper saved each penalty. A result produced by a remote match worker SHALL carry the same
record as a result produced locally for the same fixture, squads and seed.

#### Scenario: A missed penalty is recorded against its taker
- **WHEN** a player's in-play penalty is saved or goes wide during a match
- **THEN** the match result records that he missed a penalty

#### Scenario: A scored penalty is not a miss
- **WHEN** a player scores an in-play penalty
- **THEN** the match result records no missed penalty for him

#### Scenario: Local and remote results agree on misses
- **WHEN** the same fixture, squads and seed are played once locally and once on a remote match worker
- **THEN** both results record the same missed penalties

### Requirement: Fixture-derived environment and referee
Every match SHALL be played in an environment (weather, pitch condition, crowd intensity, home advantage, importance, derby intensity) derived from its fixture — date, venue, competition and the two clubs — and officiated by a referee profile that varies between matches, with the draw seeded so that the same fixture and seed always produce the same environment and referee. A friendly SHALL receive a lower importance and crowd than a competitive fixture at the same venue.

#### Scenario: Two fixtures differ in environment
- **WHEN** two matches at different venues on different dates are simulated
- **THEN** their environments and referee profiles are not required to be identical, and over a season each weather and pitch value occurs with non-zero frequency

#### Scenario: Same fixture and seed reproduce the environment
- **WHEN** the same fixture is simulated twice with the same seed
- **THEN** both runs use the same environment and referee profile

### Requirement: Environment modifiers have live consumers
Every modifier the environment declares (first touch, pass and cross accuracy, ball roll and pass speed, fatigue rate, injury risk, keeper handling, card rate) SHALL be consumed by at least one live decision or physics step, so that no declared modifier is inert in a real match.

#### Scenario: Wet pitch slows the ball and heavy legs tire faster
- **WHEN** a match is played in heavy rain on a muddy pitch
- **THEN** ground passes travel shorter, first touches fail more often and players lose condition faster than in the same match played in clear weather on a normal pitch
