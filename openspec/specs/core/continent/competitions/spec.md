# core/continent/competitions Specification

## Purpose
Defines the structural rules of continental club competitions: who qualifies from each domestic league, how the group and knockout stages are organized, and when and on which weekday their fixtures fall.

## Requirements

### Requirement: Continental qualification bands follow coefficient rank
Each continent SHALL determine, for every country ranked by reputation strongest-first, which domestic league table positions feed each continental club competition, expressed as a contiguous band of final-table places (a skip offset plus a count of places).

#### Scenario: Top-ranked European country gets four Champions League places
- **WHEN** a European country is ranked in the top 4 by reputation among continental countries
- **THEN** its top-flight league's places 1 through 4 are assigned to the Champions League qualification band

#### Scenario: A country outside the top 20 gets no Europa League band
- **WHEN** a European country's reputation rank is 20 or lower
- **THEN** no Europa League qualification band is produced for that country's league, while it may still receive a Conference League band

### Requirement: Continental competitions are scoped to their home continent
Only Europe SHALL run UEFA-style tiers (Champions League, Europa League, Conference League), and only South America SHALL run Copa Libertadores; other continents receive no continental qualification tiers.

#### Scenario: A non-European, non-South-American continent requests qualification tiers
- **WHEN** the continent is neither Europe nor South America
- **THEN** the list of continental tiers for that continent is empty

### Requirement: A country's qualifying league is its non-friendly top flight
The league that feeds continental qualification bands for a country SHALL be that country's tier-1, non-friendly league.

#### Scenario: A country has multiple divisions
- **WHEN** a country's leagues are inspected for the qualifying league
- **THEN** only the league with `tier == 1` and `friendly == false` is treated as the qualifying league; other divisions receive no qualification bands

### Requirement: Copa Libertadores field is capped
Copa Libertadores qualification SHALL be trimmed so the total number of allocated places across all qualifying countries does not exceed its fixed group-stage size of 32 clubs.

#### Scenario: Ten South American nations are ranked
- **WHEN** allocation bands are computed for ranks 0 through 9
- **THEN** the sum of allocated places equals the group-stage cap of 32

### Requirement: Continental cup group stage and knockout structure
A continental cup competition SHALL organize qualified clubs into groups of up to 4 teams playing a 6-matchday round robin (home and away against each other group member), after which the top 2 teams per group by points (then goal difference, then goals scored) advance to a two-legged knockout bracket.

#### Scenario: A qualified club field of at least 8 clubs is drawn
- **WHEN** at least 8 clubs are drawn into a competition
- **THEN** clubs are split into groups of 4 and 6 group-stage matches per group are scheduled across 6 matchdays

#### Scenario: Fewer than 8 clubs are available for a draw
- **WHEN** fewer than 8 clubs are supplied to a competition draw
- **THEN** no groups or fixtures are generated and the draw is skipped

### Requirement: Two-legged knockout ties resolve by aggregate, then shootout
A two-legged knockout tie SHALL be decided by aggregate goals across both legs (no away-goals rule applied); if aggregate is level, a supplied penalty-shootout result decides the winner; if neither resolves the tie, the winner SHALL remain undecided.

#### Scenario: Aggregate is level and a shootout result is supplied
- **WHEN** both legs finish with equal aggregate goals and a shootout score is recorded
- **THEN** the side with more shootout goals is declared the winner

#### Scenario: Aggregate is level and no shootout is supplied
- **WHEN** both legs finish with equal aggregate goals and no shootout result has been recorded
- **THEN** the tie has no winner until a shootout result is supplied

### Requirement: Continental club fixtures fall on their competition's weekday
Every continental club-competition fixture SHALL be placed on its competition's real matchday weekday. The fixture falls inside the season week that the competition's calendar names for that matchday or leg. This is the same season week grid the international windows use: week 0 begins on the first Monday on or after 29 August. The weekdays are:
- Champions League: Tuesday or Wednesday, with the draw's groups or ties split between the two days.
- Europa League and Conference League: Thursday.
- Copa Libertadores: Tuesday, Wednesday or Thursday, with groups or ties split across the three days.
- Copa Libertadores single-match final: Saturday.

No continental club fixture SHALL be dated inside an international window. At least one Saturday SHALL lie between each window's closing day and the next continental fixture. The Champions League, Europa League and Conference League matchdays that share a number SHALL fall in the same season week. Placement SHALL be deterministic for a given draw. It SHALL hold for every season year, not just years in which a nominal date happens to fall on the right weekday.

#### Scenario: A Europa League group matchday whose nominal date is a Saturday
- **WHEN** the Europa League group stage is drawn in 2026, a year in which the first matchday's former nominal date (19 September 2026) is a Saturday
- **THEN** that matchday's fixtures are dated on Thursday 17 September 2026, the Thursday of season week 2

#### Scenario: Champions League groups are drawn
- **WHEN** the Champions League group stage is drawn
- **THEN** every group-stage fixture falls on a Tuesday or Wednesday, each group plays all six matchdays on the same weekday, and both weekdays are used when there are at least two groups

#### Scenario: Knockout legs are scheduled
- **WHEN** a continental competition schedules a two-legged knockout round
- **THEN** both legs of every tie fall on the competition's weekdays, and the second leg is later than the first

#### Scenario: Placement across several seasons
- **WHEN** the same competition is drawn in 2026, 2027 and 2028
- **THEN** no group-stage or knockout fixture in any of those years falls on a Saturday or Sunday, except a Copa Libertadores single-match final

#### Scenario: No continental night inside a window
- **WHEN** every continental competition is scheduled for each season from 2026 to 2060
- **THEN** no group-stage, knockout or final fixture is dated inside an international window

#### Scenario: A free weekend after every window
- **WHEN** an international window closes on its Tuesday
- **THEN** the next continental fixture of any competition comes after at least one Saturday on which clubs can play a league round with their internationals back

#### Scenario: The three UEFA competitions share their weeks
- **WHEN** the Champions League, Europa League and Conference League schedule the same group matchday or knockout leg
- **THEN** all three fall in the same Monday-to-Sunday week

### Requirement: A club is drawn into at most one continental club competition's field
A club's league-table position SHALL place it in the qualification band of at most one continental club competition per season; the system SHALL track, per season year, every club already drawn into one of the continent's competitions so a later draw can recognize a club it has already placed.

#### Scenario: A season's draws are inspected together
- **WHEN** the set of clubs drawn this season is requested after several of the continent's competitions have completed their draws
- **THEN** it contains every club drawn into any of those competitions for that season year, deduplicated
