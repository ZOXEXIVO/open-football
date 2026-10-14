# core/match/engine/diagnostics Specification

## Purpose
Owns what the engine measures about its own realism — the per-match census exposed under the `match-logs` feature and the seeded batch acceptance test that fails when headline statistics leave their real-football bands.

## Requirements

### Requirement: Realism census coverage
Under the `match-logs` feature the engine SHALL expose, per match, every headline statistic a real match report carries and that the engine can influence: goals, shots and shots on target, xG, passes and pass accuracy, tackles, interceptions, fouls by source (tackle, holding, professional foul, dissent, time-wasting, handball), cards by source, offsides, corners, throw-ins, free kicks by type (direct, indirect), penalties awarded and scored, own goals, injuries by cause and severity, substitutions by reason, effective playing time (ball-in-play seconds), stoppage time added per period, goals by origin (open play, set piece, penalty, own goal) and goals by strike distance where a rebound finish is attributed to the rebound rather than the original strike.

#### Scenario: Rebound finish is not booked as a long-range goal
- **WHEN** a shot from outside the penalty area is parried and the rebound is finished from inside the six-yard box
- **THEN** the goal-distance census records a goal from the rebound distance, not from the original strike distance

#### Scenario: Set-piece origin is recorded
- **WHEN** a goal is scored within the same possession as a corner, free kick or throw-in without the defending side having gained controlled possession in between
- **THEN** the goal is recorded with a set-piece origin

### Requirement: Seeded batch acceptance bands
The repository SHALL contain a seeded batch test that plays a fixed number of matches between equal-strength sides and asserts the per-match averages of goals, shots, shots on target, corners, fouls, yellow cards, red cards, offsides, throw-ins, penalties awarded, effective playing time, stoppage time per period and injuries against stated real-football bands, and SHALL record every band's current value in a stored baseline so a regression is a diff, not a memory. The test MAY be `#[ignore]`d for ordinary runs but MUST be runnable on demand with one command.

#### Scenario: Statistic drifts out of band
- **WHEN** the batch test runs and the average of any band-checked statistic falls outside its stated band
- **THEN** the test fails naming the statistic, the measured value and the band

#### Scenario: Penalties are counted
- **WHEN** a penalty is awarded during a batch match
- **THEN** the batch accumulator's penalty count increases, and the penalties-per-match band is checked rather than always reading zero
