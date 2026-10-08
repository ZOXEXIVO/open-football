# core/club/player/development Specification

## Purpose
Defines how far and how fast a player's attributes may grow. Per-attribute ceilings come from his potential, his
position, his age and the football he is getting. The weekly development tick reads its minutes and rating inputs to
set the pace.

## Requirements

### Requirement: Development ceilings read maturity as a share of ability

For each attribute, the development ceiling SHALL be the position's development weight for that attribute multiplied by
a base level. That base level SHALL be the level at which the position's development shape scores `held_share × PA`.
The held share SHALL be the maturity of the attribute's family at the player's fractional age, reduced by the
family's match-only share in proportion to how little football he is getting. A ceiling SHALL be clamped to 1..20.

#### Scenario: A teenager's mind trails his peak
- **WHEN** ceilings are computed for a PA 180 central midfielder who is a regular starter, at 17½ and at 28½
- **THEN** his decisions ceiling at 17½ sits at least 4 points below the ceiling at 28½

#### Scenario: Roles reach their potential alike
- **WHEN** careers of every outfield role develop under identical conditions
- **THEN** each role ends close to the same share of its PA, because each role's ceiling base is sized to its own
  scoring weights

### Requirement: Matches open the last share of each age

Each maturation family SHALL have a match-only share of its maturity, and speed SHALL have none. The shares SHALL be
fitted against the shipped database:
- a regular starter SHALL reach his position kind's real ability-by-age spread (see "Careers track the real
  ability-by-age spread");
- a player who never plays SHALL be held below it by his families' shares.

A player SHALL be held below his age's full share by that fraction, in proportion to how little football he has
absorbed, as his match exposure measures it. A player with full exposure SHALL reach the full share.

#### Scenario: A regular starter against a squad player who only trains
- **WHEN** two otherwise identical players develop from 17 to 21, one starting every week and one never playing
- **THEN** the regular holds clearly more of his potential from 21 onward

#### Scenario: A regular who loses his place
- **WHEN** a player whose attributes sit on his regular's ceilings stops playing
- **THEN** his ceilings fall, but none of his attributes is reduced

#### Scenario: A month on the bench does not undo two seasons
- **WHEN** a player who has started every week for two seasons plays no minutes for 30 days
- **THEN** his ceilings fall by less than a tenth of the gap between a regular's ceilings and a never-played player's

#### Scenario: A keeper needs football more than an outfielder
- **WHEN** a keeper and a central midfielder of the same PA each develop from 19 to 23, once as weekly starters and
  once never playing
- **THEN** at 23 the keeper's gap in CA/PA between the starter and the unused career is larger than the midfielder's

### Requirement: Ceilings gate growth and never cut

An attribute already above its ceiling SHALL keep its value and SHALL NOT gain further until the ceiling rises past it.

#### Scenario: A prodigy above his age ceiling
- **WHEN** a 17-year-old's decisions already exceed his age ceiling
- **THEN** the development tick and training leave his decisions at their current value rather than lowering them

### Requirement: Maturation curves are continuous in age

The maturity of each family SHALL be a function of fractional age that interpolates linearly between the family's
knots and holds level outside them. No family's maturity SHALL move by more than 0.002 in any one week between ages
14 and 37. The following ordering SHALL hold:

- mental maturity trails physical maturity through the late teens
- speed arrives before strength
- speed declines before the mind does
- keepers' goalkeeping maturity peaks latest

#### Scenario: A week never moves a ceiling by a step
- **WHEN** maturity is sampled at every week from age 14 to 37 for every family
- **THEN** no consecutive pair of weeks differs by more than 0.002

#### Scenario: Maturity rises without dips through development
- **WHEN** maturity is sampled every quarter-year from 15 to 28 for the families that grow through that span
- **THEN** it never decreases

### Requirement: The development tick rates senior minutes on the 30-day window

The development tick SHALL rate a player's senior minutes against bands measured in the recency-weighted 30-day
window. A weekly 90-minute start holds about 380 minutes in that window. The useful band SHALL be:

| Age | Useful | Diminishing |
| --- | --- | --- |
| 16..17 | 200–600 | 600–1100 |
| 18..21 | 250–800 | 800–1200 |
| 22..29 | 250–850 | 850–1300 |

#### Scenario: A weekly starter at 19
- **WHEN** a 19-year-old starts one match a week
- **THEN** his 30-day minutes fall inside the useful band, not below it

### Requirement: The development rating reads league and cup games together

The rating multiplier SHALL use the player's regressed average rating over league and cup games combined, with the
combined count of official games.

#### Scenario: A youngster blooded only in the cups
- **WHEN** a player has official appearances only in cup competitions
- **THEN** his development rating comes from those cup ratings rather than being read as zero

### Requirement: Careers track the real ability-by-age spread

Seeded whole-career simulations, run through the full daily pipeline, SHALL satisfy the following:

- a career is a pure function of its inputs
- CA never exceeds PA, and attributes stay on the 1..20 scale for twenty seasons
- a regular starter of median character at a mid-table club ends every season above his position kind's database
  lower quartile for his age and below its top decile: keepers against the database's keeper spread from 22 to 30,
  outfielders against the database's outfield spread from 21 to 29
- growth slows season by season without stalling
- the best environment and character reach the top of the real spread, and the worst reach its weakest tenth
- outfielders peak in their late twenties and keepers later
- veterans decline close to the world generator's age curve for their position
- keepers outlast outfielders, and more professional players age better

#### Scenario: A typical career at 24
- **WHEN** a PA 150 outfield regular starter of median character at an average club reaches 24
- **THEN** his CA/PA sits between the database's outfield 25th and 90th percentiles for 24-year-olds

#### Scenario: A typical keeper career at 24
- **WHEN** a PA 150 regular starting keeper of median character at an average club reaches 24
- **THEN** his CA/PA sits between the database's keeper 25th and 90th percentiles for 24-year-olds (0.767 and 0.883)

#### Scenario: A young regular keeper is not ahead of the real ones
- **WHEN** a PA 150 regular starting keeper of median character at an average club reaches 22
- **THEN** his CA/PA sits below the database's keeper 90th percentile for 22-year-olds (0.819)

### Requirement: A world's keepers break through on the real timetable

A seeded multi-season world run SHALL satisfy the following from its third season on:

- keeper seasons with 30 or more league appearances SHALL have a median age between 24 and 28 (database: 26)
- no more than 15% of them SHALL belong to keepers aged 20 or younger (database: 11.2%)
- between 15% and 32% of them SHALL belong to keepers aged 22 or younger (database: 23.9%)
- outfield seasons with 30 or more league appearances SHALL keep a median age between 23 and 26 (database: 24)

Among keepers aged 17 or younger when the world starts who reach a 30-appearance league season within ten seasons,
the median age of the first such season SHALL be between 21 and 24 (database: 22).

#### Scenario: Regular keepers are mostly in their mid-twenties
- **WHEN** a world is run for five seasons and its keeper seasons with 30 or more league appearances are counted from
  the third season on
- **THEN** their median age is between 24 and 28 and no more than 15% belong to keepers aged 20 or younger

#### Scenario: A young keeper's first full season comes in his early twenties
- **WHEN** a world is run for ten seasons and every keeper aged 17 or younger at its start is followed
- **THEN** among those who reach a 30-appearance league season, the median age of the first one is between 21 and 24
