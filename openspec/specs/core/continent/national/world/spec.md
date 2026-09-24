# core/continent/national/world Specification

## Purpose
Owns the world-aware national-team pipeline: competition cycle calendars, qualifying-group scheduling, and the shared match-outcome path that updates player caps/goals/reputation, country Elo and country schedules for both continental qualifiers and global tournaments.

## Requirements

### Requirement: National-team competition cycles follow a modular calendar
Each national-team competition (World Cup, continental championship, etc.) SHALL determine whether a new qualifying cycle starts in a given year using its configured cycle length and offset, and SHALL derive the tournament year as two years after the qualifying start year.

#### Scenario: A competition configured with cycle_years=4 and cycle_offset=2 is checked in a qualifying year
- **WHEN** `(year + 2) % cycle_years == cycle_offset` holds for the given year
- **THEN** a new qualifying cycle starts that year for that competition

### Requirement: National-team match outcomes update world-wide player and country state
Every played national-team fixture, whether a continental qualifier or a global tournament match, SHALL update the same set of downstream state through one shared code path: player caps/goals/reputation, the country's Elo rating, and the country's fixture schedule, regardless of which continent hosts the match or which players are based abroad.

#### Scenario: A player based on a different continent than his national team appears in a match
- **WHEN** a player who plays his club football on a different continent from his country appears in an international fixture
- **THEN** his caps, goals, and reputation are updated exactly as they would be for a player based in his own country's continent

### Requirement: Senior international reputation gain is weighted by country strength
A player's reputation gain from a senior international appearance SHALL scale with his country's reputation, bounded between 0.4x and 2.0x of the base gain, and SHALL scale further with goals scored (capped at 3 goals counted).

#### Scenario: A player from a very high-reputation country scores in a senior international
- **WHEN** a player from a country near the top of the reputation scale scores a goal in a senior match
- **THEN** his reputation gain is weighted toward the 2.0x multiplier ceiling rather than the 0.4x floor

### Requirement: U21 appearances never touch senior caps
An Under-21 national-team appearance SHALL increment only the player's U21 caps and goals; it SHALL NOT increment senior international appearances, senior goals, or fire the senior debut event.

#### Scenario: A player plays his first match for the U21 side
- **WHEN** a player who has never played for the senior side appears in a U21 fixture
- **THEN** his U21 apps increase, his senior international apps remain unchanged, and no senior debut event fires

### Requirement: A player's first senior cap fires a debut event once
The first senior international appearance for an previously uncapped player SHALL fire a durable national-team-debut happiness event.

#### Scenario: A previously uncapped player appears in a senior fixture
- **WHEN** a player with zero senior international apps plays in a senior international match
- **THEN** a national-team debut happiness event is recorded against him

### Requirement: National-team Elo updates after every match using the opponent's live rating
After a national-team match, both countries' Elo ratings SHALL be updated using the match result and the opponent's Elo rating at kickoff.

#### Scenario: Two countries play a fixture
- **WHEN** a national-team match between two countries completes
- **THEN** each country's Elo is recalculated using its own score, the opponent's score, and the opponent's Elo rating

### Requirement: Knockout draws resolve without the away-goals rule; penalties decide unresolved ties
International-tournament knockout fixtures that finish level SHALL be resolved by a penalty shootout when the engine records one; when no shootout is available, resolution SHALL fall back to a reputation comparison rather than leaving the tie unresolved.

#### Scenario: A knockout fixture ends level and the engine recorded a shootout
- **WHEN** a national-team knockout match ends with equal score and shootout data is present
- **THEN** the side with more shootout goals advances

### Requirement: Tournament conclusion produces happiness events for every eligible squad member
When a national-team tournament concludes with a champion and a runner-up, every player who was part of the winning or runner-up squad (marked as an active international selection) SHALL receive a happiness event reflecting triumph or heartbreak; players not selected receive nothing.

#### Scenario: A country wins a major tournament
- **WHEN** a national-team competition resolves its final and a champion is recorded
- **THEN** every player on that country's squad who was marked as an active international selection receives a national-team-triumph happiness event

### Requirement: International friendlies are not simulated
International friendly fixtures SHALL remain scheduled but SHALL NOT be played through the match engine.

#### Scenario: A friendly fixture is due
- **WHEN** the simulator reaches the date of a scheduled international friendly
- **THEN** the fixture stays on the schedule but produces no simulated match result

### Requirement: National-team qualifying gives every round its own date
Every national-team qualifying group SHALL be scheduled so each round of its round-robin has its own date, and no national side (at a given team level) has two fixtures on one date.

Every qualifying date SHALL be a matchday of an international window. Each configured qualifying date names, by its month, the September, October, November or March window of the season it falls in. The configured dates that name the same window take that window's Thursday, Sunday and closing Tuesday, in their configured order. A configured qualifying date that names a month with no international window, and a window named by more than three configured dates, SHALL be rejected when the database is loaded.

When a group needs more rounds than the competition's configured qualifying dates, the calendar SHALL continue with the same windows in the following year(s). A round SHALL skip any date on which one of the group's sides already has a fixture in another campaign at the same team level. Rounds SHALL keep their order: a later round is never dated before an earlier one.

#### Scenario: The shipped qualifying calendar in 2027-28
- **WHEN** a qualifying campaign starting in 2027 uses the shipped configured dates (6 and 9 September, 11 and 14 October, 15 and 18 November, 22 and 25 March)
- **THEN** its dates are Thursday 2 and Sunday 5 September 2027, Thursday 7 and Sunday 10 October 2027, Thursday 11 and Sunday 14 November 2027, and Thursday 23 and Sunday 26 March 2028, all inside their windows and in that order

#### Scenario: A five-team group with eight configured dates
- **WHEN** a qualifying group of five sides is drawn against a calendar of eight configured dates
- **THEN** its ten rounds fall on ten different window matchdays: the eight derived from the configured dates, followed by the Thursday and Sunday of the next year's September window. No side plays twice on any date.

#### Scenario: A side already in another campaign
- **WHEN** a new qualifying campaign is drawn while one of its sides still has unplayed fixtures at the same team level in another campaign
- **THEN** none of the new group's rounds is dated on a day that side is already booked, and the new group's rounds take the next free window matchdays in order

#### Scenario: Senior and U21 campaigns on the same window
- **WHEN** a country's senior and U21 sides both have qualifiers in the same international window
- **THEN** their fixtures may share dates, because they are different sides

#### Scenario: A configured qualifying date in a month with no window
- **WHEN** the database configures a qualifying date in June for a national competition
- **THEN** loading the database fails with a message naming the competition and the month
