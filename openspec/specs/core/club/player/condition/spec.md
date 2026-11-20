# core/club/player/condition Specification

## Purpose
Defines the player's own record of recent football: competitive and friendly minute windows, the physical load
and recovery he carries from club to club, and what a day of off-season camp does to his condition.

## Requirements

### Requirement: Friendly minutes have their own window

A match's minutes SHALL be recorded in the competitive 7-day and 30-day windows only when the match is competitive.
Friendlies, youth-league games and reserve-league games SHALL be recorded only in a separate recency-weighted 30-day
friendly window, which decays on the same schedule. Rotation and selection SHALL keep reading the competitive windows
only.

#### Scenario: A youth-league start
- **WHEN** a player plays 90 minutes in a friendly-flagged youth-league game
- **THEN** his friendly 30-day window rises by 90, and his competitive 7-day and 30-day windows are unchanged

### Requirement: An off-season camp day rebuilds sharpness only

On each off-season camp day, a non-injured player's match readiness SHALL rise by `0.3 + 0.4 × facility quality`, up to
20. A camp day SHALL NOT change any technical, mental, physical or goalkeeping attribute. An injured player SHALL be left
unchanged.

#### Scenario: A summer of camps
- **WHEN** a player spends a full off-season in camps
- **THEN** his match readiness rises, and his attributes change only through the development tick and the club's
  training sessions

### Requirement: A player's load record is his own

A change of club SHALL NOT change a player's load record. His competitive and friendly minute windows, his
physical-load windows, his recovery debt and his form SHALL be the same after a transfer, a loan, a return from loan, a
release or a signing as they were before it.

#### Scenario: A transfer the day after a match
- **WHEN** a player who played 90 competitive minutes yesterday is transferred today
- **THEN** his 7-day and 30-day competitive minutes at his new club still include those 90

#### Scenario: Tired legs travel
- **WHEN** a player with heavy recovery debt moves on loan
- **THEN** his recovery debt at the borrowing club is what it was at the lending club
