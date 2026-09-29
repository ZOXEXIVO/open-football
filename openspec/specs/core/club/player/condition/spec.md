# core/club/player/condition Specification

## Purpose
Defines the player's own record of recent football: competitive and friendly minute windows, the match exposure that
development reads from them, and what a day of off-season camp does to his condition.

## Requirements

### Requirement: Friendly minutes have their own window

A match's minutes SHALL be recorded in the competitive 7-day and 30-day windows only when the match is competitive.
Friendlies, youth-league games and reserve-league games SHALL be recorded only in a separate recency-weighted 30-day
friendly window, which decays on the same schedule. Rotation and selection SHALL keep reading the competitive windows
only.

#### Scenario: A youth-league start
- **WHEN** a player plays 90 minutes in a friendly-flagged youth-league game
- **THEN** his friendly 30-day window rises by 90, and his competitive 7-day and 30-day windows are unchanged

### Requirement: Match exposure measures recent football against a regular's

A player's match exposure SHALL be computed as follows:

- add his competitive 30-day minutes to half of his friendly 30-day minutes
- divide by 330, the least a player starting every week holds in the window
- clamp the result to 0..1

#### Scenario: A weekly starter
- **WHEN** a player has started a 90-minute match every week for a month
- **THEN** his match exposure is 1.0

#### Scenario: A player who has not played
- **WHEN** a player has no competitive or friendly minutes in the window
- **THEN** his match exposure is 0.0

### Requirement: An off-season camp day rebuilds sharpness only

On each off-season camp day, a non-injured player's match readiness SHALL rise by `0.3 + 0.4 × facility quality`, up to
20. A camp day SHALL NOT change any technical, mental, physical or goalkeeping attribute. An injured player SHALL be left
unchanged.

#### Scenario: A summer of camps
- **WHEN** a player spends a full off-season in camps
- **THEN** his match readiness rises, and his attributes change only through the development tick and the club's
  training sessions
