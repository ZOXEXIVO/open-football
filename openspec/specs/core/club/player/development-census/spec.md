# core/club/player/development-census Specification

## Purpose
Defines the development census: a harness that follows every player of a stubbed world for several seasons and
reports how ability and attributes move by position, age, football played and place, beside the shipped database, so
that the balance of player development can be read from a whole world and compared between two builds.

## Requirements

### Requirement: The census follows every player of a stubbed world

The census SHALL build the shipped world, run it on the match stub, and read every player in a first team, reserve
side, youth side, academy or the free-agent pool:
- a snapshot of his age, position kind, ability, potential, place, club rank and every attribute on the opening day
  and on each anniversary of it;
- once a week, his official minutes, his friendly-flagged appearances, where he is and whether he is on loan.

It SHALL classify each player's year by his official minutes: a regular at 1,800 or more, rotation at 700 or more,
youth football below 700 with 15 or more friendly-flagged appearances, fringe with any other football, and none. A
player SHALL count as on loan for a year when he was on loan for half its weeks or more.

It SHALL group positions into nine kinds (keeper, centre-back, full-back and wing-back, defensive midfielder, central
midfielder, wide midfielder, attacking midfielder, winger, striker) and into four groups (keepers, defenders,
midfielders, forwards).

#### Scenario: A regular
- **WHEN** a player's official starts and substitute appearances over a year come to 1,800 minutes or more
- **THEN** his year is counted as a regular's

#### Scenario: A season-long loan
- **WHEN** a player is on loan for 40 of a year's 52 weekly readings
- **THEN** his year is counted as a loan

### Requirement: The census reads ability against the database

The census SHALL report, with the opening snapshot (the database as loaded) beside each later one:
- CA/PA percentiles (10th, 25th, median, 75th, 90th) by age and position group;
- the share of players at 0.98 of their potential or more, by age band and band of potential, for every year;
- the mean CA change over a year by age, position group and football played, with the first year apart from the
  later ones.

#### Scenario: The opening snapshot is the reference
- **WHEN** the census prints CA/PA by age for defenders
- **THEN** each age shows the opening world's percentiles beside the last year's

#### Scenario: The first year stands apart
- **WHEN** the census prints yearly CA change
- **THEN** the year in which the database's players first meet the model is a table of its own

### Requirement: The census reads profiles, not only ability

The census SHALL report, by position kind:
- the mean and the between-player spread of every attribute among main-team regulars aged 24 to 29, at the opening and
  at the last snapshot, naming every attribute whose mean moved by half a point or more;
- for the same men, in a first team at the opening and in a club at the end, the mean change of every attribute by
  their age at the opening;
- for men aged 28 and over at the opening, the share still in a club at the end and what they lost in pace,
  acceleration, agility, stamina, strength, jumping, passing, technique, decisions and positioning.

#### Scenario: A position drifting toward another
- **WHEN** full-backs' heading rises by three points in four seasons
- **THEN** the census names heading among the full-backs' movers, with its mean and spread before and after

### Requirement: The census answers the loan question

For players aged 17 to 22 at the start of a year with potential of 110 or more, the census SHALL report by where the
year was spent (on loan, first team, reserve side, youth side, no club) and football played: the number of players,
their mean official minutes and friendly-flagged appearances, their mean CA change, the football they banked and how
well it fitted them.

#### Scenario: Loaned regulars against those who stayed
- **WHEN** the census prints a year for 19-to-20-year-old defenders
- **THEN** regulars on loan, regulars at home, fringe players at home and youth-side players each have a row

### Requirement: The census reports where growth came from

The census SHALL report, from each player's own development record, the share of a year's attribute growth that came
from sessions, from matches and from maturation, and what ageing and lay-offs took, by age band, position group and
football played.

#### Scenario: A regular and an unused player
- **WHEN** the census prints the sources of growth for 19-to-22-year-olds
- **THEN** regulars show a share from matches and unused players show none

### Requirement: The census carries guard rows

So that a development fix cannot hide a failure elsewhere, the census SHALL also report, per year:
- players by place;
- the age profile of each group's regulars;
- first-team mean ability, mean age and head count by position kind and club tier;
- CA/PA and head count by age and place for players of 21 and under, and the football each place gives them;
- the share of 18-to-21-year-olds with 900 or more minutes of football at a standard that fits them;
- the number of players of each age from 14 to 22 with potential of 110, 130 and 150 or more.

#### Scenario: Talent supply
- **WHEN** the census prints its last year
- **THEN** it shows, for each age from 14 to 22, how many players have potential of 110, 130 and 150 or more, beside
  the opening world's count

### Requirement: Isolated careers can be printed for every position

A career grid SHALL print, for one seeded regular of each of nine positions run through the full daily pipeline: CA/PA
by age, the mean of each attribute family, how full each family is against its ceiling, the growth of each family by
source, the sessions attended, and the trajectory of every attribute. It SHALL also print the minutes ladder, the loan
paths, lost years, a season out injured, four potentials, and weekly growth by quarter-year of age from 14 to 20.

#### Scenario: Reading one position
- **WHEN** the grid is run
- **THEN** a reader can see, for a centre-back, his CA/PA at every age, every attribute at 18, 21, 25, 29, 33 and 36,
  and how much of each family's growth came from sessions and from matches

### Requirement: Two census reports can be compared

The census SHALL compare two of its reports row by row and print every figure that differs by more than the noise
between two runs of one build. Two runs of one build SHALL agree within 0.01 on every median and within one percentage
point on every share.

#### Scenario: A stage against the one before
- **WHEN** a report from before a change and one from after it are compared
- **THEN** every row that moved by more than the run-to-run noise is listed with both figures

### Requirement: A four-season census fits a development machine

A four-season census SHALL finish with resident memory below 30 GB. When free memory on the machine falls below a
floor, the census SHALL stop and report what it has measured.

#### Scenario: Four seasons
- **WHEN** the census runs four seasons of the shipped world on the stub
- **THEN** it finishes with resident memory below 30 GB and prints five yearly snapshots
