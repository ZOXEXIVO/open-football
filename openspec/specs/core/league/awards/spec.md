# core/league/awards Specification

## Purpose
Owns weekly, monthly and season award selection and their bounded retention: Player of the Week, Young Player/Team of the Week, and the monthly/season award archives.

## Requirements

### Requirement: Player of the Week award selection
Each week, the league SHALL score every candidate player's contribution across that week's matches (excluding friendlies and matches without recorded player statistics) and select a single winner, with ties broken by best single-match rating and then lowest player id for determinism.

#### Scenario: Two candidates tie on weekly score
- **WHEN** two players finish a week with equal aggregate scores
- **THEN** the winner is the one with the better single best-match rating, and if that also ties, the player with the lower id is selected

#### Scenario: Award already recorded for a calendar week
- **WHEN** the league is asked to record a Player of the Week award for a week-ending date it has already recorded an award for
- **THEN** the duplicate award is not recorded, preventing double-firing when the same in-game week is processed more than once

### Requirement: Young Player/Team of the Week apply a minimum-score floor
The Young Player of the Week and Young Team of the Week selections (restricted to players at or under the weekly age cutoff) SHALL require candidates to clear a minimum score floor; if no candidate clears the floor, no award is given that week.

#### Scenario: Weak youth pool in a low-reputation league
- **WHEN** every eligible young candidate's weekly score falls below the configured floor
- **THEN** the league records no Young Player of the Week (or no slot filled) for that week rather than crowning the best-of-a-weak-field candidate

### Requirement: Monthly and season award archives are bounded
League award history (Team of the Week, Young Team/Player of the Week, monthly snapshots, calendar-year XI, season snapshots) SHALL be retained only up to fixed retention bounds, with the oldest entries evicted as new ones are recorded.

#### Scenario: Archive at capacity receives a new entry
- **WHEN** a bounded award archive is already at its maximum retained size and a new award of that kind is recorded
- **THEN** the oldest entry in that archive is dropped so the archive size does not exceed its configured bound

### Requirement: Monthly awards skip months with no qualifying activity
A calendar month with no relevant matches or award-eligible activity SHALL NOT produce a monthly awards snapshot, so the archive's most recent entry always reflects a month that actually had fixtures.

#### Scenario: Month with no matches played (e.g. mid-off-season)
- **WHEN** a calendar month closes with zero matches contributing to the league's award pool
- **THEN** no monthly awards snapshot is recorded for that month, and the archive's "latest" entry remains the last month that did have fixtures
