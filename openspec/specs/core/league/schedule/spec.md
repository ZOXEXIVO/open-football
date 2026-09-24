# core/league/schedule Specification

## Purpose
Owns fixture generation and placement for every domestic competition shape: round-robin league rounds, knockout cup pairing and dates, kickoff times, calendar-day resolution, development-competition timing, and rescheduling around continental fixtures and international windows.

## Requirements

### Requirement: Round-robin fixture generation covers every ordered pairing exactly once
League schedule generation SHALL produce a double round-robin in which every team plays every other team exactly twice — once at home, once away — with no missing or duplicated pairings, using a bye placeholder to keep round sizes fixed when the team count is odd.

#### Scenario: Even number of teams in a division
- **WHEN** a league schedule is generated for an even-sized division
- **THEN** every ordered (home, away) pair among the division's teams occurs exactly once across the full season

#### Scenario: Odd number of teams in a division
- **WHEN** a league schedule is generated for an odd-sized division
- **THEN** each round pads the field with a bye so one team rests that round, and the fixed per-round match count is preserved

### Requirement: League rounds stop for international windows and fit the season window
Every league round SHALL be dated outside the international windows. This holds for every league: every country, every tier, senior and development. Rounds SHALL fall on the league's round weekday (Saturday for senior, Friday for development). The first round is the first round weekday on or after the season opening day. The last round SHALL be on or before the league's configured closing day.

When there are fewer window-free round weekdays between the first round and the closing day than the league has rounds, the league SHALL add midweek rounds (Tuesday for senior, Wednesday for development) to make up the difference. Midweek rounds SHALL fall in window-free weeks strictly between the first and the last weekend round. Weeks without continental club football SHALL be used before any week with it, and the midweek rounds SHALL be spread evenly across the weeks they use. A league whose weekends suffice SHALL NOT get midweek rounds.

If weekend and midweek rounds together still cannot fit the season window, the remaining rounds SHALL continue after the closing day on window-free round weekdays, so every round is still scheduled. A split-season league SHALL apply this rule to each tournament within that tournament's own window. Round placement SHALL be deterministic: regenerating the same season yields the same dates.

#### Scenario: A 20-club league in 2027-28
- **WHEN** a senior league of 20 clubs that opens on 16 August and closes on 25 May generates its 2027-28 schedule
- **THEN** its 38 rounds fall on 36 Saturdays and 2 Tuesdays, no round is dated inside an international window, and the last round is on or before 25 May 2028

#### Scenario: Every season from 2026 to 2060
- **WHEN** the same 20-club league generates each season from 2026 to 2060
- **THEN** every season has 38 rounds, each dated on a Saturday or a Tuesday, none inside an international window, and none after 25 May

#### Scenario: A league with slack in its calendar
- **WHEN** a league has at least as many window-free Saturdays between its first round and its closing day as it has rounds
- **THEN** it plays only on Saturdays, on the earliest window-free Saturdays from the first round day

#### Scenario: Midweek rounds and continental weeks
- **WHEN** a league needs two midweek rounds and has more window-free weeks without continental club football than that
- **THEN** both midweek rounds fall in weeks without continental club football, so a club playing on a continental Thursday is not also given a league Tuesday that week

#### Scenario: A 24-club league with a May close
- **WHEN** a senior league of 24 clubs that opens on 9 August and closes on 3 May generates its schedule
- **THEN** all 46 rounds are dated on or before 3 May, none inside an international window, and the midweek rounds it needs are Tuesdays in window-free weeks

#### Scenario: A season window too small for its rounds
- **WHEN** a league's season window cannot hold all its rounds even with midweek rounds
- **THEN** every round is still scheduled, the rounds after the closing day fall on window-free round weekdays, and no round is dated inside a window

#### Scenario: A development league
- **WHEN** a development league generates its schedule
- **THEN** its rounds fall on Fridays, plus Wednesdays only if the Fridays run short. No round is dated inside an international window, and none is dated before the day the schedule is generated.

#### Scenario: A split-season league
- **WHEN** a split-season league generates its schedule
- **THEN** no first-tournament round is dated after the first tournament's closing day, no second-tournament round is dated before the second tournament's opening day, and no round in either half is inside an international window

#### Scenario: A club's internationals are away
- **WHEN** players of a club are on international duty for a window
- **THEN** that club has no league fixture on any day of the window, and it plays its league round on the weekends either side of the window with those players available

### Requirement: Knockout cup pairing and round shaping
A knockout cup round SHALL pair seeded entrants strongest-vs-weakest, awarding byes to the top seeds when the entrant count is not a power of two, so the field reduces to the next lower power of two each round.

#### Scenario: Cup round with a non-power-of-two entrant count
- **WHEN** a knockout round begins with a number of teams that is not a power of two
- **THEN** only enough ties are played to reduce the field to the next lower power of two, and the surplus top seeds receive byes into the next round

#### Scenario: Cup tie decided by penalties
- **WHEN** a knockout tie ends level after regulation and extra time
- **THEN** the tie's winner is resolved from the penalty shootout tally rather than the regulation/extra-time score

### Requirement: Cup fixtures avoid league fixture days
Domestic cup ties SHALL be scheduled on midweek dates (snapped to the next Wednesday) with an evening kickoff, so a club is never scheduled for both a league fixture and a cup fixture on the same day. No domestic cup tie and no grouped-competition playoff game SHALL be dated inside an international window. When a cup round's midweek date or a playoff game's date falls inside a window, it SHALL move to the first Wednesday after the window closes. A cup tie that is later moved around a continental commitment or a league fixture SHALL still never share a day with either team's league fixture, and SHALL never move into an international window.

#### Scenario: Cup round falls on a Saturday
- **WHEN** a cup round's computed date would land on a Saturday, the day the round-robin league scheduler uses
- **THEN** the date is advanced to the following Wednesday before the tie is scheduled

#### Scenario: A moved cup tie
- **WHEN** a cup tie is moved off a continental matchday
- **THEN** its new day holds no league fixture of either participating team

#### Scenario: A cup round's midweek falls inside an international window
- **WHEN** a cup round's computed Wednesday is inside an international window
- **THEN** the round is dated on the first Wednesday after that window's closing Tuesday

#### Scenario: Cup rounds over many seasons
- **WHEN** a cup of one to eight rounds is scheduled for every season from 2026 to 2060, both for an August-to-May season window and for a calendar-year season window
- **THEN** no round is dated inside an international window

#### Scenario: A playoff series is scheduled
- **WHEN** a grouped-competition playoff series places its games
- **THEN** no game of the series is dated inside an international window

### Requirement: Fixtures carry a realistic kickoff day and time
Every domestic fixture the scheduler produces (league round, domestic cup tie, grouped-competition playoff game) SHALL carry a kickoff time inside the playing day rather than midnight. Senior weekend fixtures SHALL kick off in the afternoon or evening. Senior midweek and weekday fixtures SHALL kick off in the evening. Development (youth) fixtures SHALL kick off in the late morning or around midday. Kickoffs within one day SHALL be staggered across that day's slots deterministically, so regenerating the same schedule yields the same kickoffs.

#### Scenario: A senior league round is generated
- **WHEN** a senior league's season schedule is generated
- **THEN** every weekend-round fixture is dated on its round's Saturday with a kickoff between 12:00 and 21:00, every midweek-round fixture is dated on its round's Tuesday with a kickoff between 18:00 and 21:00, and no fixture is dated 00:00

#### Scenario: A domestic cup round is drawn
- **WHEN** a cup round is drawn onto its midweek date
- **THEN** each tie carries an evening kickoff between 18:00 and 21:00 on that date

#### Scenario: Fixtures in one round are staggered
- **WHEN** a round holds more fixtures on one day than that day has kickoff slots
- **THEN** the fixtures are spread across all of the day's slots in a stable order, and regenerating the schedule assigns each fixture the same kickoff again

### Requirement: Matchday fixtures are resolved by calendar day
A competition SHALL play every unplayed fixture whose kickoff falls on the simulation's current calendar day, whatever the kickoff's time of day, while the simulation clock itself continues to advance in whole days.

#### Scenario: A fixture kicks off mid-afternoon
- **WHEN** the simulation ticks the day whose date matches a fixture scheduled for 15:00
- **THEN** that fixture is built and played on that tick and is never skipped because the clock reads 00:00

#### Scenario: A round spans two days
- **WHEN** fixtures of the same round fall on a Saturday and on the following Sunday
- **THEN** the Saturday fixtures are played on the Saturday tick and the Sunday fixtures on the Sunday tick, each exactly once

### Requirement: Development competitions play the day before the senior round
A development (youth) competition SHALL play its weekend rounds on Fridays, the day before the senior Saturday rounds, starting on the first Friday on or after the season start. When it needs midweek rounds, it SHALL play them on Wednesdays, the day after the senior Tuesday rounds. A club's youth fixtures SHALL therefore never fall on the same calendar day as its senior league fixtures under the default calendar, and a youth round is never placed before the day its schedule is generated.

#### Scenario: A youth league and its parent league generate schedules
- **WHEN** a U18 or U19 league and the senior league it was created from generate their schedules for the same season
- **THEN** every youth fixture is dated on a Friday or a Wednesday, and no youth fixture shares a calendar day with any fixture of the parent league

#### Scenario: The season starts on a Saturday
- **WHEN** the season window opens on a Saturday and the youth league's schedule is generated that day
- **THEN** the first youth round is dated the following Friday, the day before the senior league's second round, and no youth round is dated before the season start

#### Scenario: A youth fixture's kickoff
- **WHEN** a development fixture is placed on its day
- **THEN** its kickoff falls between 10:00 and 14:00

### Requirement: Domestic fixtures give way to continental commitments
When a club's first team has a continental fixture, any of that team's unplayed domestic fixtures (league, cup or playoff) falling within the minimum rest gap of it SHALL be moved. An unplayed domestic cup tie or playoff game falling within the minimum rest gap of any other first-team fixture of either side (a league fixture or another knockout tie) SHALL also be moved. A league fixture keeps its date, because league fixtures give way only to continental commitments. The minimum rest gap is two clear days between matches, so a Thursday tie is followed at the earliest by a Sunday game. The fixture SHALL move to the nearest day that gives both participating teams the minimum rest from every other first-team fixture they have. If no nearby day achieves the full gap, it SHALL move to the day that maximises the smaller of the two teams' rest. A day inside an international window SHALL never be chosen, even when it would be the nearest rested day. A moved fixture SHALL keep its identity, round, and home/away sides; SHALL receive a kickoff appropriate to its new day; and SHALL never be moved to a date earlier than the current simulation day. Fixtures already clear of the gap SHALL NOT move.

#### Scenario: A Europa League Thursday before a Saturday league game
- **WHEN** a club plays a Europa League tie on Thursday and its league fixture is on the following Saturday
- **THEN** the league fixture moves to Sunday with a Sunday kickoff, and both clubs play it on Sunday

#### Scenario: A Champions League Tuesday after a Saturday league game
- **WHEN** a club plays its league fixture on Saturday and a Champions League tie the following Tuesday
- **THEN** the league fixture keeps its Saturday date because the gap already allows two clear days

#### Scenario: A cup tie falls on a continental matchday
- **WHEN** a club's domestic cup tie is scheduled on the same Wednesday as its Champions League tie
- **THEN** the cup tie moves to the nearest day on which both cup teams have two clear days from all their other first-team fixtures, and the club is never scheduled for two first-team fixtures on one day

#### Scenario: The opponent also has a continental commitment
- **WHEN** a league fixture involves two clubs that each play a continental tie that week, on different days
- **THEN** the chosen day gives both clubs the minimum rest from their own continental tie, or, if no nearby day can, maximises the smaller of their two rest gaps

#### Scenario: A reschedule would reach into the past
- **WHEN** the best rest-respecting day for a clashing fixture lies before the current simulation day
- **THEN** the fixture is only considered on days from the current day onwards

#### Scenario: A cup tie the day after a midweek league round
- **WHEN** a club plays a league fixture on Tuesday and a domestic cup tie on the following Wednesday
- **THEN** the cup tie moves to the nearest day that gives both cup teams two clear days from all their other first-team fixtures, and the league fixture keeps its Tuesday date

#### Scenario: A cup round drawn the day after the previous one
- **WHEN** a club's cup tie was played on Tuesday and its next-round tie is drawn for the following Wednesday
- **THEN** the new tie moves to the nearest day that gives both sides two clear days from all their other first-team fixtures, and the played tie keeps its date

#### Scenario: The nearest rested day is a window's closing Tuesday
- **WHEN** the nearest day that would rest both sides falls inside an international window
- **THEN** the fixture moves to the nearest rested day outside every window instead
