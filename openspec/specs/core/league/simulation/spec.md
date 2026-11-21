# core/league/simulation Specification

## Purpose
Owns the matchday build/process pipeline internals: squad-selection importance, matchday squad backfill across a club's own sides, and non-matchday processing including season-end.

## Requirements

### Requirement: Squad selection importance drives rotation
For every fixture a club plays, the league simulation SHALL compute an importance score between 0.0 (dead rubber) and
1.0 (must-win) for each side separately, from what that side is playing for. The score determines how strong a lineup
the club selects.

For a league fixture the score SHALL be a continuous reading of how much this result can still change for the side:
its distance in points from every line its league really has (the title, each continental qualification band,
promotion, a play-off place, relegation), measured against the points it can still win. A line the side can no
longer cross in either direction SHALL count for nothing. The lines SHALL be read from the league's own rules and the
continental qualification bands, not from fixed table positions. A competitive league fixture SHALL keep a floor of
professional importance, so that a side with nothing left to play for rotates but does not field a development team.
A derby SHALL raise the score.

For a cup tie the score SHALL rise continuously with the round reached and with the strength of the opponent
relative to the side, and a final SHALL be a must-win.

The score SHALL NOT be lowered because another fixture follows. What the rest of the week asks of a club is a
separate reading.

#### Scenario: Team with nothing left to play for
- **WHEN** a club's league position guarantees neither European qualification, promotion, nor relegation risk for the remainder of the season
- **THEN** the computed importance for its remaining fixtures drops to the professional floor, allowing some reserve and youth players to be selected beside a core of regulars

#### Scenario: Domestic cup final
- **WHEN** a fixture is a domestic cup final
- **THEN** its importance is that of a must-win whatever else is in the club's week

#### Scenario: The visitors are fighting relegation
- **WHEN** a side one point above the relegation line with four matches left visits a mid-table side that can neither go up nor down
- **THEN** the visitors' importance is near the top of the scale and the home side's is at the professional floor

#### Scenario: A chase for a European place
- **WHEN** a side sits seventh, two points from the last continental place with six matches left, and cannot reach the top three
- **THEN** its importance is high

#### Scenario: A lower division's promotion race
- **WHEN** a second-division side is level on points with the last promotion place late in the season
- **THEN** its importance is high, read from that league's promotion places

#### Scenario: A champion already crowned
- **WHEN** a side has won the title with three matches left
- **THEN** its importance for those matches is at the professional floor

#### Scenario: No cliff in the table
- **WHEN** two sides differ by one point at the same stage of the season
- **THEN** their importance scores differ by a small amount

### Requirement: Matchday squad backfill from club depth
When a team cannot field a full matchday squad from its own registered players, the simulation SHALL backfill from the same club's other eligible teams (senior reserves and older youth sides) before a fixture is played, respecting availability (injury, suspension, international duty). A team SHALL NOT borrow any player whose own team has a fixture on the same calendar day, in any competition. This rule applies alike to senior call-ups, emergency goalkeeper and shortfall borrowing, youth sides' supplementary players, and overage development players. On a day when several of a club's sides have fixtures, only the most senior of them borrows from the club's other squads, in pathway order: first team, then Second/B/Reserve, then the youth sides from oldest to youngest. The club's other playing sides field only their own registered players. Together these rules ensure no player is named in two matchday squads on one day.

The one exception is a player assigned to another side on the day before: a first-team player released to a side below, or a goalkeeper assigned to a side that has none. For that day he belongs to the side he was assigned to and to no other, his own side does not count him, and the side he joins may name him even when his own side plays.

#### Scenario: Team short a fit goalkeeper
- **WHEN** a team lacks enough available goalkeepers to cover a starter and a substitute
- **THEN** the assembling process borrows the best available academy goalkeepers from deeper youth tiers, preferring the oldest eligible tier first, and never borrows an unavailable (injured/banned/on international duty) player

#### Scenario: Youth team short on numbers
- **WHEN** a U18 or U19 team needs match practice players beyond its own squad
- **THEN** older overage youth players from higher tiers (U20/U21/U23) may be pulled in under a position-aware quota that reserves a distinct slot for goalkeepers

#### Scenario: B team plays on the same day as the first team
- **WHEN** a club's first team and its B/"2" squad both have fixtures on the same Saturday
- **THEN** no player registered to the B/"2" squad is offered to the first team's matchday pool that day, and each of those players can appear only in the B/"2" squad's match

#### Scenario: B team is idle on the first team's matchday
- **WHEN** the club's B/"2" squad has no fixture on the first team's matchday
- **THEN** its available players are offered to the first team's matchday pool exactly as before

#### Scenario: Youth side on its own matchday
- **WHEN** a youth side assembles its squad on a day when another of the club's teams also has a fixture
- **THEN** no player from that other team is offered to the youth side as a supplementary or overage player

#### Scenario: First team and B side share a Saturday
- **WHEN** a club's first team and its B/"2" side both play on the same day and its U20 squad has no fixture
- **THEN** the U20 players may be offered to the first team, the B/"2" side borrows nobody that day, and no U20 player is named in both squads

#### Scenario: Academy player the day after his youth fixture
- **WHEN** the first team plays on the day after a youth fixture
- **THEN** the youth side's available players are eligible for the first team's call-up sweeps, because their own team has no fixture that day

#### Scenario: A player assigned the day before
- **WHEN** a first-team player was released to the B/"2" side on Friday for its Saturday fixture, and the first team also plays on Saturday
- **THEN** he is in the B/"2" side's matchday pool, he is not in the first team's, and he is named in one squad only

### Requirement: Season-end processing freezes standings and awards
When a league reaches its configured season-ending date, it SHALL record the champion (top table row), snapshot the season's awards before statistics are archived, freeze a final table (the annual aggregate for split-season leagues, otherwise the raw table), reset season-scoped dynamics, archive season statistics, and clear season-scoped disciplinary state (active suspensions, yellow-card accumulation, pending disciplinary cases).

#### Scenario: Standard (non-split) league reaches season end
- **WHEN** the current date matches the league's configured season-ending day/month
- **THEN** the champion is recorded from the current top table row, the final table snapshot equals the live table at that moment, and outstanding yellow-card accumulations are cleared going into the new season

### Requirement: A club's week is read across competitions
For every competitive fixture, the simulation SHALL give selection the club's week: the days since its last
competitive fixture and until its next, in any competition, and what that next fixture is worth to the club. The
reading SHALL be the same whichever competition's fixture is being built.

#### Scenario: A league match before a European tie
- **WHEN** a club plays a league match on Saturday and a continental tie on Tuesday
- **THEN** Saturday's selection is told a fixture follows in three days and what it is worth

#### Scenario: A continental tie before a league match
- **WHEN** the same club's Tuesday tie is built
- **THEN** its selection is told a league match follows on Saturday and what it is worth

#### Scenario: A free week
- **WHEN** a club's nearest competitive fixtures are a week either side
- **THEN** selection is told the week is free

### Requirement: A first-team player short of football plays for the club's next side
On the day before a fixture of one of a club's other sides, the first team's manager SHALL release to that side the
first-team players who:
- are eligible for that side's competition;
- are behind the football the plan gives them, or are returning from an absence and short of match fitness;
- are not wanted for a first-team fixture on that day or on the day either side of it.

A released player SHALL be offered to that side for the fixture and SHALL NOT be named by any other side of the club
on that day. The players released to one fixture SHALL be few enough that the side's own players keep most of its
shirts. A manager's release SHALL be decided before that day's squads are picked.

#### Scenario: A promoted prospect
- **WHEN** a 19-year-old on the first-team roster has not played for three weeks and the club's B side plays on a day the first team does not
- **THEN** he is offered to the B side for that fixture

#### Scenario: A man coming back from injury
- **WHEN** a first-team regular returns from two months out and the reserve side plays before the first team's next fixture
- **THEN** he is offered to the reserve side

#### Scenario: Needed by the first team
- **WHEN** the first team plays the next day with a thin squad
- **THEN** no first-team player it would name is released

#### Scenario: Not eligible
- **WHEN** the only other side with a fixture is an under-19 side and the player is 24
- **THEN** he is not released to it

#### Scenario: One squad per day
- **WHEN** a player is released to the B side on a day the first team also plays
- **THEN** he appears in the B side's squad only

### Requirement: Every side is found a goalkeeper
On the day before a fixture, a side that would have no fit goalkeeper SHALL be assigned one from another of the
club's sides, the most senior side choosing first. The assignment SHALL hold even when the goalkeeper's own side
plays on the same day, provided that side keeps a fit goalkeeper of its own. This SHALL apply to friendlies and
development fixtures as it does to competitive ones. A side SHALL start an outfield player in goal only when the club
has no fit goalkeeper it can assign.

#### Scenario: The first team loses both keepers
- **WHEN** a first team's two goalkeepers are injured and its under-21 side, which has two fit goalkeepers, plays on the same day
- **THEN** one of the under-21 goalkeepers starts for the first team and the other for the under-21 side

#### Scenario: A youth friendly
- **WHEN** an under-18 side's only goalkeeper is injured before a friendly and the under-19 side has no fixture that day
- **THEN** an under-19 goalkeeper is assigned to the under-18 side

#### Scenario: No keeper anywhere
- **WHEN** a club has no fit goalkeeper in any of its sides
- **THEN** the side starts the outfield player its coaches judge most able to keep goal
