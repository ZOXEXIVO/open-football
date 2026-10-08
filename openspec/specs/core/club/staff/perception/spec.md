# core/club/staff/perception Specification

## Purpose
The perception capability supplies every staff-observable read of a player —
assessed current ability, development ceiling and biased impressions — built only
from evidence a coach could actually see.

## Requirements

### Requirement: Staff perception classifies a player's current level from observable evidence, not hidden ability
The staff perception layer SHALL derive a player's assessed current level from evidence a coach can actually observe — his visible, position-weighted skill, his match-results record once he has a meaningful sample, his training application, and his reputation — and SHALL NOT read the player's hidden current-ability value directly.

#### Scenario: Early-season player with a thin match sample
- **WHEN** the season has not produced enough matches to assess a player's results-based evidence
- **THEN** the perception estimator falls back to his visible skill and reputation rather than treating the small sample as evidence of being worse, so any consumer built on it never penalises idle minutes

### Requirement: Staff perceive a player's state of mind from observable evidence
A staff member's view of a player's state of mind (how assured he is at a standard, how confident and how nervous he
is) SHALL be built only from evidence the staff member could observe:
- his errors;
- his willingness to act, such as claims attempted, risks taken and passes played forward;
- his match ratings;
- where and how much he has been playing.

The view SHALL NOT read the player's assurance, self-belief, morale or nerves directly:
- **Judgement:** its accuracy SHALL improve with the staff member's judgement and with the number of observations.
- **Determinism:** its noise SHALL be deterministic for a given staff member, player and week.
- **Silence:** with no observations to go on, the view SHALL be silent (zero confidence) rather than a confident
  neutral reading.

#### Scenario: A better judge reads him more accurately
- **WHEN** two staff members with different judgement observe the same player over the same matches
- **THEN** the better judge's perception of his state of mind is, on average over many players, closer to the
  player's actual state

#### Scenario: Nothing seen, nothing claimed
- **WHEN** a staff member has no observations of a player who has not played in the period
- **THEN** his perception of that player's state of mind reports zero confidence

#### Scenario: Errors are noticed
- **WHEN** a keeper makes errors leading to goals in consecutive matches
- **THEN** the staff's perceived confidence of that keeper falls
