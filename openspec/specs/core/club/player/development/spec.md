# core/club/player/development Specification

## Purpose
Defines how a player's attributes change, in one place: the profile he grows toward, per-attribute ceilings from
his potential, his age and the football he has banked, the work sessions and matches do, what age and idle weeks
take back, and the record of every change by source.

## Requirements

### Requirement: Development ceilings read maturity as a share of ability

For each attribute, the development ceiling SHALL be the player's blueprint weight for that attribute multiplied by a
base level. That base level SHALL be the level at which his blueprint scores `held_share × PA` for his position. The
held share SHALL be the maturity of the attribute's family at his fractional age, read at his own pace of maturing,
reduced by the family's match-only share in proportion to the football he has not banked. Past the age ageing starts,
each attribute's ceiling SHALL carry that attribute's own retention at his care, in place of its family's mean. A
ceiling SHALL be clamped to 1..20.

#### Scenario: A teenager's mind trails his peak
- **WHEN** ceilings are computed for a PA 180 central midfielder who is a regular starter, at 17½ and at 28½
- **THEN** his decisions ceiling at 17½ sits at least 4 points below the ceiling at 28½

#### Scenario: Roles reach their potential alike
- **WHEN** careers of every outfield role develop under identical conditions
- **THEN** each role ends close to the same share of its PA, because each player's ceilings are sized to his own
  blueprint and his position's scoring weights

### Requirement: Matches open the last share of each age

Each maturation family SHALL have a match-only share of its maturity, and speed SHALL have none. The shares SHALL be
fitted against the shipped database:
- a regular starter SHALL reach his position group's real ability-by-age spread (see "Careers track the real
  ability-by-age spread");
- a player who never plays SHALL finish near its weakest tenth.

A player SHALL be held below his age's full share by that fraction, in proportion to the football he has not banked
(see "Football is banked for a career, by how well it fits him"). A player whose football is fully banked SHALL reach
the full share.

#### Scenario: A regular starter against a squad player who only trains
- **WHEN** two otherwise identical players develop from 17 to 28, one starting every week and one never playing
- **THEN** at 28 the one who never played holds at least 0.10 of his potential less if he is an outfielder, and at
  least 0.15 less if he is a keeper

#### Scenario: A career without a match
- **WHEN** an outfielder of median character at an average club trains from 17 to 28 and never plays
- **THEN** his CA/PA at 28 is between 0.65 and 0.82

#### Scenario: A regular who loses his place
- **WHEN** a player whose attributes sit on his regular's ceilings stops playing
- **THEN** his ceilings do not fall, and none of his attributes is reduced by it

#### Scenario: A month on the bench does not undo two seasons
- **WHEN** a player who has started every week for two seasons plays no minutes for 30 days
- **THEN** the football he has banked is unchanged, and so are his ceilings apart from what his age moves

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

The maturity of each family SHALL be a function of fractional age that runs between the family's knots along a
monotone cubic, its slope continuous everywhere and flat at the first and last knot, and holds level outside them. No
family's maturity SHALL move by more than 0.002 in any one week between ages 14 and 37. The following ordering SHALL
hold:

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
- a regular starter of median character at a mid-table club ends every season between his position group's database
  lower and upper quartiles for his age: keepers from 22 to 30, and defenders, midfielders and forwards from 21 to 29,
  each against its own group's spread
- growth slows season by season without stalling
- the best environment and character reach the top of the real spread, and the worst reach its weakest tenth
- outfielders peak in their late twenties and keepers later
- veterans hold the share of their peak that "Age takes a share of what a player has, pace first" bounds
- keepers outlast outfielders, and more professional players age better

#### Scenario: A typical career at 24
- **WHEN** a PA 150 outfield regular starter of median character at an average club reaches 24
- **THEN** his CA/PA sits between his position group's database 25th and 75th percentiles for 24-year-olds (defenders
  0.810 and 0.891, midfielders 0.814 and 0.891, forwards 0.825 and 0.906)

#### Scenario: A typical keeper career at 24
- **WHEN** a PA 150 regular starting keeper of median character at an average club reaches 24
- **THEN** his CA/PA sits between the database's keeper 25th and 75th percentiles for 24-year-olds (0.767 and 0.862)

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

### Requirement: A player's attributes change only through his own development

After a player is created, his technical, mental, physical and goalkeeping attributes SHALL change only through his
own development. Training sessions, academy sessions, matches and mentoring SHALL report what happened, and his
development SHALL decide what changes. Match readiness is a condition, not an attribute, and is outside this rule.

#### Scenario: The academy and a youth squad are one rule
- **WHEN** two identical 16-year-olds do the same session at the same quality, one in an academy and one in a youth
  squad
- **THEN** their attributes change by the same amounts

#### Scenario: Nothing else writes an attribute
- **WHEN** the source of every module other than the player's development, the generators and profile fitting is
  checked
- **THEN** none of it assigns a player's attribute

### Requirement: A player grows into his own profile

Each player SHALL have a blueprint: a target for every attribute that keeps the relative strengths of the profile he
was created or loaded with, projected to maturity, and that scores exactly his potential for his position. His
ceilings and his growth SHALL follow his blueprint. They SHALL NOT follow a profile shared by the players of his
position.

An individual training plan SHALL tilt the blueprint toward its attribute by a bounded amount.

#### Scenario: The blueprint is worth his potential
- **WHEN** a profile sitting on every target of a player's blueprint is scored for his position
- **THEN** it scores within 1 point of his potential

#### Scenario: A winger stays a winger
- **WHEN** a right winger whose crossing is above his finishing at 18 develops to 29 as a regular
- **THEN** his crossing is still above his finishing

#### Scenario: A full-back does not become a centre-back
- **WHEN** a PA 150 left-back who starts at 18 with crossing 12 and heading 10 develops to 29 as a regular
- **THEN** his heading is still below his crossing, and none of his marking, tackling and positioning has reached 20

#### Scenario: Targets keep his order
- **WHEN** a player's blueprint is built
- **THEN** within each attribute family its targets stand in the order of his attributes

#### Scenario: Two centre-backs stay different
- **WHEN** two centre-backs of the same potential develop from 18 to 28 under the same conditions, one starting 3
  points quicker and the other 3 points better in the air
- **THEN** at 28 the first is still at least 2 points quicker and the second at least 2 points better in the air

#### Scenario: Character is not a position's
- **WHEN** a winger whose aggression and bravery are his two lowest mental attributes at 18 develops to 29
- **THEN** they are still his two lowest mental attributes, apart from any that started within one point of them

#### Scenario: A specialty plan polishes its skill
- **WHEN** a player follows an individual plan for one attribute for a season
- **THEN** that attribute ends higher than it would have without the plan, and his ability does not exceed his
  potential

### Requirement: Attributes grow from the work done on them

An attribute SHALL grow only from work:
- training sessions that practise it;
- match football in a position that uses it;
- for the physical and explosive families, the maturing of a body that is still growing.

Work SHALL count for more at a more plastic age, from a better session, and for a more professional, more ambitious
and fresher player. The same work SHALL move players of different potential by the same share of their own range.
Growth SHALL slow as an attribute nears its ceiling and stop at it. An attribute a player does not have (zero) SHALL
stay absent.

#### Scenario: A player with no club
- **WHEN** a 22-year-old without a club spends a year with no session and no match
- **THEN** none of his technical or mental attributes rises

#### Scenario: What the squad trains matters
- **WHEN** two identical defenders train for a season, one with his unit's defending sessions and one without them
- **THEN** the first ends the season with higher tackling and marking

#### Scenario: Better coaching, more growth
- **WHEN** two identical 19-year-olds below their ceilings do the same sessions for a season, one under a coach of 17
  and one under a coach of 6
- **THEN** the first gains more

#### Scenario: A regular's growth has named sources
- **WHEN** a regular develops from 18 to 24
- **THEN** sessions and matches together account for at least three quarters of his attribute growth, and each of them
  for at least a quarter

#### Scenario: An outfielder's goalkeeping stays absent
- **WHEN** an outfielder whose goalkeeping attributes are zero trains for a season
- **THEN** they are still zero

### Requirement: Football is banked for a career, by how well it fits him

Every match SHALL add to the football a player has banked, for each maturation family, in proportion to:
- his minutes;
- how well the match's standard of football fits his own standard: most for football near his standard or a little
  above it, less for football far above it, and least for football far below it;
- how plastic he is at his age, so the same football banks less the older he is.

Banked football SHALL NOT fade and SHALL NOT change when he changes club. Friendlies, youth-league and reserve-league
games SHALL be credited by the same rule, on their own standard. Academy sessions SHALL bank the games they contain at
the academy's standard.

#### Scenario: A loan beats the youth side once he has outgrown it
- **WHEN** a PA 160 prospect whose standard is above his club's youth league spends the seasons from 18 to 21 in one of
  three ways: starting on loan for a senior side whose football fits him, starting for the youth side, or unused in
  the first team
- **THEN** at 22 the loan leaves him at least 0.02 of his potential above the youth side and at least 0.05 above the
  bench

#### Scenario: Youth football suits a boy
- **WHEN** a 16-year-old plays a season in a youth league whose standard is close to his own
- **THEN** his football is banked at the full rate for his age

#### Scenario: Overmatched
- **WHEN** a 16-year-old plays senior football far above his standard
- **THEN** each minute banks less than a minute of football at his own standard

#### Scenario: A loan leaves its mark
- **WHEN** a player returns to his club after a season on loan
- **THEN** his banked football is what it was on the last day of the loan

#### Scenario: A lifetime of youth football
- **WHEN** a player plays only youth-league football from 17 to 28
- **THEN** at 28 he holds at least 0.06 of his potential less than a senior regular under the same conditions

### Requirement: Football missed young is only partly made up

A player who plays little while his families are maturing SHALL end below a player who played throughout, and SHALL
recover part of the difference, not all of it, once he plays.

#### Scenario: Bench years cost
- **WHEN** two identical prospects become first-team regulars at 22, one after four seasons as a regular elsewhere and
  one after four seasons unused
- **THEN** at 26 the second is still at least 0.02 of his potential below the first

#### Scenario: Eight lost years
- **WHEN** an outfielder plays no match until 25 and starts every week from then on
- **THEN** at 30 he is at least 0.04 of his potential below a regular since 17, and at least 0.03 above a player who
  never played

### Requirement: Potential is reached alike at every level, and rarely in full

Under identical conditions, players of different potential SHALL hold the same share of their potential at each age. A
regular of median character at an average club SHALL peak between 0.90 and 0.96 of his potential.

#### Scenario: A low and a high potential
- **WHEN** a PA 90 and a PA 180 regular develop from 17 to 35 under identical conditions
- **THEN** their CA/PA differ by no more than 0.03 at any age from 21 to 35

#### Scenario: An ordinary regular's peak
- **WHEN** a PA 150 regular of median character at an average club reaches his best season
- **THEN** his CA/PA is between 0.90 and 0.96

### Requirement: Age takes a share of what a player has, pace first

Past an attribute's decline age, a player SHALL lose each week a share of that attribute's level, not a fixed number
of points. The share SHALL depend on the attribute and his age, fitted to the shipped database's attributes by age:
- pace and acceleration go first and furthest;
- agility and stamina follow;
- strength, jumping, balance and technique barely move;
- no mental attribute falls before 34.

Natural fitness and professionalism SHALL slow the loss. A player's position SHALL NOT change it.

#### Scenario: A regular from 29 to 35
- **WHEN** a PA 150 outfield regular of median natural fitness and professionalism ages from 29 to 35
- **THEN** his pace and his acceleration each fall by 2 to 4 points, his agility and his stamina each by 1 to 3, his
  strength, jumping and balance each by no more than 1, each technical attribute by no more than 1, and no mental
  attribute falls

#### Scenario: Ability at 35
- **WHEN** a regular reaches 35
- **THEN** an outfielder holds between 0.85 and 0.95 of his peak CA, and a keeper at least 0.93

#### Scenario: No attribute collapses
- **WHEN** a player reaches 38
- **THEN** none of his attributes is below half its highest value

#### Scenario: The same share at every potential
- **WHEN** a PA 90 and a PA 180 regular reach 35 under identical conditions
- **THEN** the shares of their peak CA they hold differ by no more than 0.03

#### Scenario: A professional ages better
- **WHEN** two identical regulars reach 35, one with natural fitness and professionalism of 18 and one with 6
- **THEN** the first holds more of his peak CA

### Requirement: Time passes for a player who is not working

The weekly development SHALL run for every player, whether he is fit, injured or without a club. A player who does no
session and plays no match SHALL gain nothing from them and SHALL age like any other. After four weeks without work
his physical and explosive attributes SHALL give back a small share of their level, no more than a tenth, which
training SHALL return faster than it was first built.

#### Scenario: A season out at 32
- **WHEN** a regular misses a whole season injured at 32 and then plays on
- **THEN** at 33 his CA is below a healthy twin's, and at no later age is it above

#### Scenario: A season out at 20
- **WHEN** a regular misses a whole season injured at 20 and then plays on
- **THEN** at 21 his CA is below a healthy twin's, at 26 his CA/PA is within 0.02 of the twin's, and at no age is his
  CA above the twin's

#### Scenario: A long injury costs sharpness of body
- **WHEN** a 24-year-old returns from six months out
- **THEN** his pace and strength are lower than on the day he was hurt, and within six months of training they are
  back within 0.3 of it

### Requirement: Players mature at their own pace

Each player SHALL have a timing: how far ahead of or behind his age he matures, within a year and a half either way.
Every age-dependent part of development SHALL read his age moved by his timing. The timing SHALL fade to nothing
between 22 and 26.

A generated or academy player SHALL draw his timing when he is created. A player loaded from the database SHALL get the
timing that puts his recorded ability on his own curve, within the same bounds.

#### Scenario: An early and a late developer
- **WHEN** two otherwise identical 17-year-olds develop as regulars, one maturing a year early and one a year late
- **THEN** at 19 the first is at least 0.04 of his potential ahead, and at 27 they are within 0.02 of each other

#### Scenario: A recorded prodigy keeps growing
- **WHEN** a world is built containing an 18-year-old recorded at 0.75 of his potential, and he plays the season as a
  regular
- **THEN** his ability rises during that season

### Requirement: No birthday changes the pace of development

Every age-dependent quantity in development SHALL be continuous in fractional age.

#### Scenario: The sixteenth birthday
- **WHEN** a youth-side regular is followed week by week from 14 to 20
- **THEN** at each birthday his mean weekly growth over the 13 weeks after it is within 15% of the 13 weeks before it

### Requirement: A player's development can be read back

For the current season and the one before, the system SHALL keep each player's attribute change by source (sessions,
matches, maturation, ageing, lay-off) and by family, the football he banked, and how well that football fitted him.

#### Scenario: A season adds up
- **WHEN** a season ends
- **THEN** the sum of the player's recorded changes equals the change in his attributes over the season, within
  rounding

#### Scenario: An unused player
- **WHEN** a squad player trains all season and plays no match
- **THEN** his record shows growth from sessions and none from matches

### Requirement: A world's players start with the careers they have had

When a world is built, every player SHALL be given:
- football banked from every season in his record, credited at the age he played it, or from his squad role when he
  has no recorded season;
- a timing, as "Players mature at their own pace" requires;
- a blueprint from his recorded profile.

A player created during play SHALL start with no banked football, a drawn timing and a blueprint from his generated
profile.

#### Scenario: A regular on the world's first day
- **WHEN** a world is built containing a 25-year-old with 30 or more appearances in each of his last five recorded
  seasons
- **THEN** his football is fully banked on the first day

#### Scenario: A third keeper who has never played
- **WHEN** a world is built containing a 24-year-old keeper with no recorded appearance who ranks third at his club
- **THEN** less than a fifth of his football is banked

### Requirement: A world keeps the database's shape

A seeded world run for four seasons on the stub SHALL satisfy the following at its last yearly snapshot. Ratios are
read among players in club squads with potential of 110 or more, by position group, against the shipped database.

- The median CA/PA at each age from 22 to 33 SHALL be within 0.03 of the database's.
- The median CA/PA at each age from 34 to 36 SHALL be within 0.05 of the database's.
- The share of players aged 25 to 29 at 0.98 of their potential or more SHALL be between 5% and 15% in each band of
  potential (under 110, 110 to 139, 140 and over).
- Among main-team regulars aged 24 to 29 of each position kind, no attribute's mean SHALL differ from the database's
  by more than 1.5, no more than three by more than 1.0, and the spread of attributes between players SHALL be within
  10% of the database's.
- Men aged 22 to 27 in a top-500 first team at the start SHALL gain between 3 and 8 CA over the four seasons.
- Regulars aged 19 to 22 SHALL gain at least twice the CA of squad members of the same age who played no football, and
  players without a club no more than a quarter of it.
- For regulars aged 22 to 30, the mean CA change of the first season SHALL be within 1.5 of later seasons' at the same
  age.

#### Scenario: Ability does not pile up on potential
- **WHEN** the census reads its last snapshot
- **THEN** between 5% and 15% of 25-to-29-year-olds stand at 0.98 of their potential or more in each band of potential,
  and each group's median CA/PA from 22 to 33 is within 0.03 of the database's

#### Scenario: Veterans are still footballers
- **WHEN** the census reads its last snapshot
- **THEN** each group's median CA/PA at 34, 35 and 36 is within 0.05 of the database's

#### Scenario: Positions keep their identity
- **WHEN** the census compares main-team regulars aged 24 to 29 of each position kind with the database's
- **THEN** no attribute's mean has moved by more than 1.5, at most three by more than 1.0, and the spread between
  players is within 10% of the database's

#### Scenario: Football shows in the world
- **WHEN** the census reads a year's CA change among 19-to-22-year-olds
- **THEN** regulars gained at least twice what unused squad members gained, and players without a club at most a
  quarter of a regular's gain

#### Scenario: The database meets the model without a jump
- **WHEN** the census compares the first season's CA change with later seasons' for regulars aged 22 to 30
- **THEN** they differ by no more than 1.5 at any age
