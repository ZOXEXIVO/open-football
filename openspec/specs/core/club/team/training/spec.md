# core/club/team/training Specification

## Purpose
Defines when a club's squad trains and how the weekly training plan reads the fixture calendar: no training on match
days, and lighter plans only in genuinely congested weeks.

## Requirements

### Requirement: No training session runs on a match day

A club's plan for a date SHALL contain no sessions when that date is its next fixture. It SHALL also contain no
sessions when that date is its previous fixture, because the league plays before the clubs train.

#### Scenario: The day's match is already played
- **WHEN** a club plans its day after today's match has been filed as its previous fixture
- **THEN** today's plan has no sessions

### Requirement: Congestion is two competitive fixtures in the week around today

A week SHALL count as congested when the club has two or more competitive fixtures, played or upcoming, within three
days either side of today. When the league-side fixture window has not been written yet, the fallback count SHALL be
matches in the last seven days.

#### Scenario: One match a week
- **WHEN** a club plays only on Saturdays
- **THEN** no day of the season is planned as a double-match week

#### Scenario: A midweek fixture
- **WHEN** a club plays Saturday, Wednesday and Saturday
- **THEN** the days around the midweek fixture are planned as a double-match week

### Requirement: Each position unit practises its own craft

On every training day that is not the day after a match, a club's plan SHALL give each unit a session of its own
craft:
- keepers: goalkeeping;
- defenders: tackling, marking, heading and positioning;
- midfielders: passing, ball control or decision making;
- attackers: shooting, for every attacking position.

Wide players of any unit SHALL practise crossing at least once a week. Team sessions (shape, pressing, transition, set
pieces, match preparation, recovery) SHALL stay for the whole squad.

#### Scenario: A keeper's one-match week
- **WHEN** a club plays only on Saturdays
- **THEN** a keeper attends at least four goalkeeping sessions in the week

#### Scenario: A defender's one-match week
- **WHEN** a club plays only on Saturdays
- **THEN** a centre-back attends at least two sessions that practise tackling and marking

#### Scenario: A winger's one-match week
- **WHEN** a club plays only on Saturdays
- **THEN** a winger attends at least one shooting session and at least one crossing session

#### Scenario: A congested week still has unit work
- **WHEN** a club plays Saturday, Wednesday and Saturday
- **THEN** every unit has at least one session of its own craft between the three matches

### Requirement: No attribute a position relies on goes unpractised

Over any two weeks of an ordinary season, a player SHALL attend at least one session that practises each attribute his
position's ability scoring gives real weight to. In season, when the week is not congested, the plan SHALL hold one
session of strength or agility work, and speed work SHALL practise acceleration as well as pace.

#### Scenario: Every position, every relied-on attribute
- **WHEN** the plan for two ordinary one-match weeks is read for a player of each exact position
- **THEN** every attribute his position's scoring weights above the mean weight appears in a session he attends

#### Scenario: Strength in season
- **WHEN** a club plays only on Saturdays in mid-season
- **THEN** the week holds one strength or agility session

### Requirement: A session reports its work and does not change attributes

For each participant a session SHALL report the repetitions it gave each attribute and their quality: the coach who
took him, the facility, how well he executed the session, the squad's chemistry and his bond with the coach.
Repetitions SHALL rise with intensity and duration, less than in proportion. The report SHALL NOT depend on the
player's age or potential, and the session SHALL NOT change any attribute itself.

A session SHALL keep deciding its fatigue cost, physical load, injury risk, readiness gain and morale effect.

#### Scenario: The same session for a boy and a veteran
- **WHEN** an 18-year-old and a 33-year-old execute the same session equally well under the same coach
- **THEN** the work reported for them is the same, and what each gains from it is decided by his own development

#### Scenario: A double-length session does not teach double
- **WHEN** the same session runs for 45 minutes and for 90 minutes
- **THEN** the longer one reports more repetitions, and fewer than twice as many
