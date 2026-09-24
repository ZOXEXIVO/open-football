# core/match/engine/rating Specification

## Purpose
Computes post-match player ratings, including the goalkeeper's goals-prevented model, from each player's recorded stat line and match context.

## Requirements

### Requirement: Post-match player rating
The system SHALL compute a per-player match rating from that player's recorded stat line, position group, and team-outcome context (such as a clean sheet or goals conceded), weighting individual contributions by zone of the pitch and by minutes played so that a short late cameo is not scored as heavily as a full match on the same raw stat totals.

#### Scenario: Short substitute appearance dampens event bonuses
- **WHEN** a substitute plays only the closing minutes of a match and records a small number of standout actions
- **THEN** the rating contribution from those actions is dampened relative to the same actions recorded over a full 90 minutes

#### Scenario: Clean sheet credit is evidence-gated for defenders
- **WHEN** a defender's team keeps a clean sheet but that defender's own recorded defensive actions in dangerous zones are minimal
- **THEN** the defender receives a reduced clean-sheet rating bonus rather than the full credit given to a defender with clear defensive involvement

### Requirement: Goalkeeper rating from shot-stopping evidence
The system SHALL compute the goalkeeper's match rating from the chance value of shots faced, saved, or conceded, so it measures goals prevented above expectation rather than raw save volume.

#### Scenario: Post-shot xG feeds the keeper's rating
- **WHEN** a shot on target is saved or scored
- **THEN** the chance value of that shot is recorded against the keeper's match stats and used to compute goals-prevented above expectation
