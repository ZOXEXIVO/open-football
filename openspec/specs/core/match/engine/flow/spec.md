# core/match/engine/flow Specification

## Purpose
Owns the match clock and the produced match result — the scoreline, its derived outcome, highlights, and each player's end-of-involvement physical snapshot.

## Requirements

### Requirement: Match clock and stoppage time
The match clock SHALL advance in fixed increments for the duration of each timed period, and SHALL accumulate additional (stoppage) time within a period up to a bounded cap rather than extending a period indefinitely.

#### Scenario: Stoppage accumulates during a period
- **WHEN** the engine records stoppage time during the first or second half or extra time
- **THEN** that period's effective end time is extended by the recorded amount, up to the period's stoppage cap

#### Scenario: Stoppage not recorded outside a timed period
- **WHEN** a stoppage-time contribution is recorded while the match is in half time, a shootout, or has ended
- **THEN** it has no effect on any period's length

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
