# core/awards Specification

## Purpose
Selects and records individual and team football awards (weekly, monthly, yearly and season-end) from match performance data, and applies their reputation and happiness side effects to the winning players.

## Requirements

### Requirement: Weekly Player of the Week selection
The system SHALL select a Player of the Week per non-friendly league every Monday, based on an aggregate score computed from that league's matches played in the preceding calendar week, and SHALL NOT select more than one winner per league per week.

#### Scenario: League already has an award for the week
- **WHEN** the weekly award tick runs for a league that already recorded a Player of the Week for the current week-ending date
- **THEN** the league is skipped and no new award is produced for that week

#### Scenario: Friendly league excluded
- **WHEN** the weekly award tick evaluates a league marked as friendly
- **THEN** no Player of the Week is selected for that league

### Requirement: Young Player of the Week eligibility
The system SHALL restrict the Young Player of the Week award to players at or under a configured maximum age on the award date, and SHALL require the winning candidate's score to meet a minimum threshold before a winner is recorded.

#### Scenario: No eligible young candidate meets the score floor
- **WHEN** every player under the age ceiling in a league's weekly aggregate scores below the young-award minimum score
- **THEN** no Young Player of the Week is recorded for that league that week

#### Scenario: Eligible young player wins
- **WHEN** a player at or under the age ceiling has the highest qualifying score and it meets the minimum threshold
- **THEN** that player is recorded as the league's Young Player of the Week for the week

### Requirement: Team of the Week and Young Team of the Week selection
The system SHALL assemble a weekly XI per league using fixed formation quotas, and a separate Young Team of the Week restricted to under-age-ceiling players, gated by a minimum candidate score rather than a minimum appearance count.

#### Scenario: No candidates clear the formation quotas
- **WHEN** a league's weekly candidate pool cannot fill the required XI slots
- **THEN** no Team of the Week award is recorded for that league that week

#### Scenario: Thin single-fixture week still produces a Young XI
- **WHEN** a league played only one fixture in the week and young candidates clear the minimum score floor
- **THEN** a Young Team of the Week can still be selected without requiring multiple appearances

### Requirement: Monthly awards run once per calendar month for the previous month
The system SHALL evaluate Player of the Month, Young Player of the Month, and a monthly snapshot (team of month, young team of month, top scorers, top assists, best ratings) only on the first day of a calendar month, covering the previous calendar month's matches.

#### Scenario: Not the first of the month
- **WHEN** the current simulated date is not day 1 of the month
- **THEN** the monthly awards tick takes no action

#### Scenario: League had no qualifying matches in the previous month
- **WHEN** a league recorded zero non-friendly matches with stats in the previous calendar month
- **THEN** the league is skipped entirely — no Player of the Month, no snapshot, and its last-monthly-award marker is not updated

### Requirement: Monthly Player of the Month and Young Player of the Month selection
The system SHALL select a Player of the Month from all qualifying candidates in a league using an appearance-gated selector, and a Young Player of the Month restricted to players aged 21 or under on the award date, using a lower minimum appearance requirement.

#### Scenario: Young winner also wins senior award
- **WHEN** the same player qualifies as both Young Player of the Month and Player of the Month winner in the same league and month
- **THEN** the Young award's reputation impact is applied before the senior award's, so the senior emission is dampened relative to the young one

### Requirement: Monthly statistical leaderboards
The system SHALL produce ranked top-5 lists of goal scorers, assist providers, and average-rating leaders for each league's monthly snapshot, ordered by the relevant stat with deterministic tie-breaking.

#### Scenario: Tied goal totals
- **WHEN** two players have equal goals scored in the monthly window
- **THEN** the ranking breaks the tie by assists, then by average rating, then by player id, to produce a stable order

#### Scenario: Minimum appearances for best-rating leaders
- **WHEN** a player has fewer matches played than the monthly minimum-appearances threshold
- **THEN** that player is excluded from the best-ratings leaderboard regardless of rating

### Requirement: Team of the Year selection runs once per calendar year
The system SHALL select a calendar-year XI per non-friendly league on year-end, built from all matches played between January 1 and December 31 of that year, gated by a per-player minimum-appearances threshold scaled to the league's actual fixture density that year.

#### Scenario: Split-season league with a thin fixture count
- **WHEN** a league's calendar-year window contains fewer matches than a typical full campaign
- **THEN** the minimum-appearances gate scales down proportionally rather than using a fixed threshold, while never falling below a floor of 10 appearances

#### Scenario: League already has a Team of the Year for this year
- **WHEN** the league already recorded a Team of the Year for the current year
- **THEN** the tick skips that league

### Requirement: Season-end awards are drained from a per-league pending snapshot
The system SHALL apply season-end awards (Player of the Season, Young Player of the Season, Team of the Season, top scorer, top assists, golden glove) only for leagues that have a pending season-awards snapshot staged by season-end processing, and SHALL clear that snapshot once applied.

#### Scenario: League has no pending season snapshot
- **WHEN** a league has not staged a pending season-awards snapshot
- **THEN** no season-end awards are applied for that league on this tick

#### Scenario: Same player wins Young and senior Player of the Season
- **WHEN** a player qualifies for both the young and senior season awards in the same league
- **THEN** the young award's reputation impact is applied first so the senior award's impact is dampened

### Requirement: World Player of the Year selection pools all continents
The system SHALL run once per year-end, rank players across every continent using a shared per-continent ranking method, and select the global top three as nominees with the single highest-ranked player as the winner.

#### Scenario: Not year-end
- **WHEN** the current simulated date is not the last day of the calendar year
- **THEN** the world player-of-year tick takes no action

#### Scenario: Winner and runner-up context
- **WHEN** a world player-of-year winner is determined
- **THEN** the winner's recognition event carries the runner-up identity and the score margin between winner and runner-up

### Requirement: Award winners receive reputation and happiness side effects
The system SHALL notify each award-winning player through its own reaction mechanism and apply a reputation impact scoped to the award kind, using league reputation, average rating, and matches played as inputs when available.

#### Scenario: Winner has no resolvable club at award time
- **WHEN** an award winner cannot be resolved to a current club roster entry
- **THEN** the award still records with empty club identifying fields rather than failing

### Requirement: Weekly award computation is cached and shared across weekly ticks
The system SHALL compute each non-friendly league's weekly aggregate statistics once per Monday and share that computed result across the Player of the Week, Young Player of the Week, Team of the Week, and Young Team of the Week selections for that week.

#### Scenario: Reused cache avoids recomputation
- **WHEN** all four weekly award ticks run for the same Monday
- **THEN** each league's underlying match aggregation for that week is computed only once, not once per tick
