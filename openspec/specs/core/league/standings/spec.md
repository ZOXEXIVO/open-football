# core/league/standings Specification

## Purpose
Owns the league's derived-from-results state that is not the table itself: aggregate statistics and rankings, and per-team momentum/rivalry/race dynamics.

## Requirements

### Requirement: Match results update league statistics and player rankings
Processing a finished match SHALL update the league's aggregate statistics (goals, results distribution, competitive-balance signal) and refresh top-scorer, top-assist, and clean-sheet rankings, scoped strictly to players on teams competing in that league.

#### Scenario: Country with multiple divisions
- **WHEN** a country has a Tier-2 striker with more goals than the Tier-1 league's leading scorer
- **THEN** the Tier-1 league's top-scorer ranking is computed only from Tier-1 teams and does not surface the Tier-2 player

### Requirement: Team momentum and rivalry dynamics update after each match
After each match result, the league SHALL update each team's momentum (a bounded rolling form signal), win/loss streak counters, and derby/rivalry recognition, and periodically refresh title-race, relegation-battle, and European-qualification-race framing from the current table.

#### Scenario: Team on a winning run
- **WHEN** a team wins several consecutive matches
- **THEN** its momentum value and winning-streak counter both increase, and its losing-streak counter resets to zero
