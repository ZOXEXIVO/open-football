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
