# core/league/news Specification

## Purpose
Owns the league's own monthly newspaper: publication cadence, its rolling archive, and deriving story content strictly from the already-frozen monthly awards snapshot.

## Requirements

### Requirement: League newsroom publishes one edition per month
A league's own newspaper SHALL publish at most one edition per calendar month, retaining a rolling twelve-month (one-year) archive of past editions, with the oldest edition evicted when a new one is published beyond capacity.

#### Scenario: Thirteenth monthly edition published
- **WHEN** a league newsroom that already holds twelve editions publishes a new monthly edition
- **THEN** the oldest of the twelve is dropped so exactly twelve editions remain on the shelf

#### Scenario: Month already closed
- **WHEN** the newsroom is asked whether a given month has already been covered and that month was previously published or explicitly closed with no content
- **THEN** it reports the month as covered, so the same month's edition is not attempted twice

### Requirement: Monthly league news derives from the frozen awards snapshot
The league's monthly news content (scoring chart, assists chart, ratings chart, team-of-month feature) SHALL be produced strictly by reading the already-frozen `MonthlyAwardsSnapshot` for that month, never by recomputing chart data independently from match records.

#### Scenario: Monthly awards snapshot has already been computed
- **WHEN** the news desk files stories for a month whose awards snapshot exists
- **THEN** every chart and story printed uses only the figures already present in that snapshot, so the league's news page and its awards page can never disagree with each other
