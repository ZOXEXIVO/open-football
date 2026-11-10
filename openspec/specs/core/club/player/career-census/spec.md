# core/club/player/career-census Specification

## Purpose
Defines the career census: a development harness that follows cohorts of different kinds of player through whole
careers in a stubbed world, so the realism of a career can be read as a biography and counted as a rate.

## Requirements

### Requirement: The census follows cohorts of different kinds of player

The census SHALL select cohorts from the world shortly after clubs have first assigned squad status, by facts about
each player and his club, and follow every selected man until the run ends or he retires. The cohorts SHALL be:

- elite stars: prime-age regulars at the biggest clubs
- prospects at those clubs
- wonderkids at smaller clubs
- ambitious climbers at mid-sized clubs
- settled regulars of ordinary ambition at mid-sized clubs
- fringe players at big clubs
- club servants: loyal men with long service at one club
- unsettled foreigners
- veterans still regular at big clubs
- lower-league professionals
- backup goalkeepers at big clubs
- free agents

Selection SHALL NOT depend on a random draw. No man SHALL belong to two cohorts.

#### Scenario: Cohorts are not empty because statuses are unset

- **WHEN** the census starts on a freshly generated world
- **THEN** it selects after the first squad-status pass, and every cohort holds men

#### Scenario: A run follows a man to the end

- **WHEN** a followed man moves club, goes out on loan, becomes a free agent or retires
- **THEN** he stays in his cohort and his biography records it

### Requirement: The census prints a biography a reader can follow

For a sample of each cohort the census SHALL print a dated biography. It SHALL hold one line a year (age, ability,
reputation, wage, club and its rank, division, role, share of starts, contract left, last season's appearances, goals
and rating, plan, strongest wants, and dream once he has one) and one line per event (each move with its fee and
reason, each request and withdrawal with its reasons, each listing, renewal and refusal, each dream turn, and
retirement).

#### Scenario: Reading one career

- **WHEN** a reader opens the biography of a followed star
- **THEN** every move shows the club and division on both sides, the fee, the recorded reason, whether he was listed or
  asking, and what he wanted at the time

### Requirement: The census counts the moves a reader would question

The census SHALL report each of these as a count and, where a denominator applies, as a rate per 100 player-seasons, by
club size, role and age band, naming the worst cases:

- regulars who left for nothing, split into contract expiry, pay-off and other routes
- regulars paid off while still starting
- regulars dropping a division or more
- players released and re-signed by the same club within a year
- players making two permanent moves within a year
- players returning within a year to the club that sold them
- transfer requests per man, requests that lasted under a fortnight, and requests with no reason
- listings of regulars, by reason, and men still listed after their own request ended
- plans to step up held at the biggest clubs
- moves to a rival of a club the player counts as a favourite
- retirements of men still regular

It SHALL also report how long wants and plans last and what ended them, how many dreams were lived, revised or let go,
and, by club size over time, the world's reputation, wages and fees, with the highest of each.

It SHALL report guard rows so a fix cannot hide a new failure: moves per season by type, squads short of a position,
the size of the free-agent pool and the average stay on it.

#### Scenario: A rate has a denominator

- **WHEN** the census reports regulars leaving for nothing at elite clubs
- **THEN** it gives the count, the player-seasons at risk and the rate per 100

### Requirement: The census shows whether clubs judge players by what they did

The census SHALL report, among regulars of similar ability at clubs of similar size, how a season's rating relates to
the following season's role, new contract and departure.

#### Scenario: A good season matters

- **WHEN** regulars of similar ability are split by their season rating
- **THEN** the census reports each group's share promoted in role, renewed, listed and sold the following season

### Requirement: The census can compare a world with and without the anchors

One build of the census SHALL be able to run with the career anchors disarmed, so their effect can be measured on the
same code. Disarmed, decisions SHALL read as if a player held no anchors.

#### Scenario: One build, two arms

- **WHEN** the census is run with the anchors disarmed
- **THEN** it reports the same rows, and no decision reads a summit or a dream

### Requirement: A whole-career run fits on a development machine

A run of twelve simulated seasons SHALL complete on a 64 GB machine. The census MAY shorten what the world keeps of past
matches to achieve this, and SHALL print its resident memory at each report.

#### Scenario: Twelve seasons

- **WHEN** the census runs for twelve simulated seasons
- **THEN** it finishes with resident memory below 50 GB
