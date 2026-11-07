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
The repository SHALL contain a seeded batch test that plays a fixed number of matches between equal-strength sides and asserts the per-match averages of goals, shots, shots on target, corners, fouls, yellow cards, red cards, offsides, throw-ins, penalties awarded, effective playing time, stoppage time per period, injuries, state changes per player per minute and distance covered per outfield player against stated bands, and SHALL record every band's current value in a stored baseline so a regression is a diff, not a memory. The test MAY be `#[ignore]`d for ordinary runs but MUST be runnable on demand with one command.

#### Scenario: Statistic drifts out of band
- **WHEN** the batch test runs and the average of any band-checked statistic falls outside its stated band
- **THEN** the test fails naming the statistic, the measured value and the band

#### Scenario: Penalties are counted
- **WHEN** a penalty is awarded during a batch match
- **THEN** the batch accumulator's penalty count increases, and the penalties-per-match band is checked rather than always reading zero

#### Scenario: State churn is band-checked
- **WHEN** a change raises outfield state changes per player per minute above 45
- **THEN** the batch report shows that statistic out of band and diffs it against the baseline

#### Scenario: Distance covered is reported against the real band
- **WHEN** the batch runs
- **THEN** it reports distance covered per outfield player per 90 minutes against a real band of 10 to 13 km, so the distance is tracked as a diff from the stored baseline even while it remains out of band

### Requirement: State machine census
Under the `match-logs` feature the engine SHALL report, per match, how players move through their states, in enough detail that an oscillation cannot hide behind a limit on its rate. The report SHALL include:
- state changes per player per minute, the share that reverse the previous change and the share within one AI tick of entry;
- for every observed transition, its source, its count per match and a histogram of how long the player had been in the state he left;
- the count of cycles through three states;
- the count of transitions a commitment rule held back or the resolution refused, by pair of states;
- the count of times each transition site in the code proposed a change, including proposals that did not take effect;
- for every state, its time occupied, the share of that time with the ball, the share standing still, the mean speed and the effort tier it was served at.

#### Scenario: A rate-limited oscillation is visible
- **WHEN** two states exchange a player as fast as a commitment rule allows
- **THEN** the dwell histogram for that transition shows the concentration at the rule's limit, and the held-back count for that pair is reported

#### Scenario: A cycle of three states is visible
- **WHEN** players cycle through three states with none of them reversing the previous change
- **THEN** the census reports that cycle by name, although no two-state reversal occurred

#### Scenario: A noisy door can be found
- **WHEN** one transition site proposes far more changes than any other
- **THEN** the census names its source location and its proposals per match

#### Scenario: A foul is attributed to the state it was decided in
- **WHEN** a marking defender commits a foul and the same decision moves him to another state
- **THEN** the foul census attributes the foul to the state he was marking in, and not to the state he moved to

### Requirement: Possession reading census
Under the `match-logs` feature the engine SHALL report, per match, how often players' readings of possession change, how often they change while the ball is unowned, how often team-mates disagree, and how often both sides or neither side claim possession.

#### Scenario: A disagreement is counted
- **WHEN** two team-mates read possession differently on the same tick
- **THEN** that tick is counted as a disagreement in the census
