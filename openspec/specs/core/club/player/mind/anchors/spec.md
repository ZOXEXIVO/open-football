# core/club/player/mind/anchors Specification

## Purpose
Defines the career anchors a player carries for his whole life in the game: how high he has been, what he is reaching
for, and the dated turning points of his career. They are the long-term facts his decisions rest on, and nothing about
a change of club erases them.

## Requirements

### Requirement: A player carries his career anchors through every move

A player's anchors SHALL survive every kind of move: a sale, a loan in either direction, a release, a free signing,
and a stay in the free-agent pool. They SHALL stay readable after he retires. No club-change reset SHALL touch them.

#### Scenario: A sale keeps them

- **WHEN** a player with a dream and a summit is sold
- **THEN** he holds the same dream and summit at the new club the next day

#### Scenario: A retired player's career can still be read

- **WHEN** a player retires
- **THEN** his final dream state, summit and chapters remain readable

### Requirement: A player's summit is the highest level he has been a regular at

A player SHALL hold a summit: the strongest division he has been a regular in, on the league-reputation scale, with the
club, the date he reached it and the date he was last a regular at that level or above.

He SHALL count as a regular only once enough of his matches have been seen, on his share of starts. A role on paper
SHALL NOT count, and neither SHALL a cameo. A season as a regular on loan SHALL count at the division he played in.
Falling to a lower division SHALL NOT lower his summit.

Decisions SHALL read a held summit, not the raw record. A held summit SHALL stand in full for a year after he was last
a regular at that level, then fade to nothing over the following years, and faster the older he is. A summit he cannot
hold, because he no longer looks like a player of that level, SHALL fade the same way.

#### Scenario: Starting every week in La Liga

- **WHEN** a player has started most of a season's matches for a club in a 9300 division
- **THEN** his summit is 9300, at that club, on the date enough matches were seen

#### Scenario: A squad player has no summit there

- **WHEN** a player sits on the bench of a 9500 club for a whole season with a handful of cameos, having been a regular
  in a 6000 division before
- **THEN** his summit stays 6000

#### Scenario: A loan counts for what it was

- **WHEN** a 20-year-old starts most of a season on loan in a 7000 division
- **THEN** his summit is at least 7000, whatever division his parent club plays in

#### Scenario: A fall does not lower it

- **WHEN** a 27-year-old regular in a 9000 division moves to a 6000 division
- **THEN** his summit stays 9000 and his held summit stands in full for the first year

#### Scenario: It fades

- **WHEN** a player has not been a regular at his summit's level for five years and is now 33
- **THEN** his held summit no longer stands above the division he plays in

### Requirement: A move up is counted in his career

A permanent move or a loan to a stronger level SHALL be recorded as a step up in his career's trajectory, the way
honours and setbacks already are. A move to a weaker level as a regular SHALL count as a setback.

#### Scenario: A real step up lifts his read of his career

- **WHEN** a player moves from a 6000 division to a regular's place in an 8000 division
- **THEN** his career trajectory rises

### Requirement: A player's dream forms from who he is and how good he looks

Every player old enough to have a senior career ahead of him SHALL hold one dream. It SHALL be one of:

- **reach the top:** the strongest level he can see himself starting at, as a club standing
- **play for his club:** a named club he comes from or has adored, when he is not there
- **belong here:** to be the man at the club where he is
- **finish at home:** to end his career in his own country
- **make a career:** to keep playing at a level that pays, with no higher aim

Which one SHALL follow continuously from his ambition, loyalty and adaptability, his observable level and ceiling, his
age, his years at the club, and whether he is abroad. It SHALL NOT follow from a random draw at the moment it is
formed. The dream SHALL carry a target level and a conviction from 0 to 1. For reach the top, the target SHALL be the
strongest club standing at which his projected level would start, raised by his ambition and his youth. The same rule
SHALL apply at every level of the game.

#### Scenario: An ambitious prospect at a giant

- **WHEN** a 19-year-old with very high ambition and a world-class ceiling is formed
- **THEN** his dream is to reach the top, with a target at the very strongest club standing

#### Scenario: An ambitious boy in a small league

- **WHEN** a 19-year-old with very high ambition and a modest ceiling in a low league is formed
- **THEN** his dream is to reach the top, with a target above his club and well short of the world's biggest clubs

#### Scenario: A content professional

- **WHEN** a 26-year-old regular with low ambition is formed
- **THEN** his dream is to make a career

#### Scenario: A loyal long server

- **WHEN** a 29-year-old with very high loyalty and eight years at his club is formed
- **THEN** his dream is to belong here

#### Scenario: An unsettled man late in his career abroad

- **WHEN** a 31-year-old abroad with low adaptability is formed
- **THEN** his dream is to finish at home

### Requirement: A dream changes only at a review, and says why

A dream SHALL be reviewed once a season and otherwise left alone. A change of club, a bad month or a want SHALL NOT
change it.

At a review, conviction SHALL fall with every window he wanted to move in and nobody came, and rise with evidence that
he is getting closer. The target SHALL be lowered when his level and remaining career no longer reach it, and raised
when his level has passed it. Ambition SHALL make him slower to lower it. When a dream can no longer be held it SHALL be
replaced by the one his situation now argues for. Each change SHALL be recorded with its date and its reason.

#### Scenario: Nobody came

- **WHEN** an ambitious 27-year-old has wanted the top for years, has not improved and no big club has come
- **THEN** a review lowers his target, and in time he makes his peace with a career at his level, on a dated chapter

#### Scenario: The late bloomer

- **WHEN** a 24-year-old whose dream was to make a career has improved far beyond his club's level
- **THEN** a review replaces it with reaching the top

#### Scenario: A move does not change it

- **WHEN** a player reaching for the top is sold to a smaller club
- **THEN** his dream is unchanged and is further away

### Requirement: A player knows when he has arrived

A player SHALL read as having arrived when he is a regular at the level or the club his dream names. Arriving SHALL be
recorded once, with its date. While he has arrived, he SHALL have no bigger stage to want. He SHALL stop reading as
having arrived when he stops being a regular there or leaves, and the dream SHALL then be alive again, as something he
has had and wants back.

#### Scenario: A star at Real Madrid

- **WHEN** a 22-year-old with a dream of the very top is starting nearly every match for one of the world's biggest
  clubs
- **THEN** he has arrived

#### Scenario: A big fish in his own pond

- **WHEN** a low-league player's dream names a target his club has reached, and he starts for it
- **THEN** he has arrived, by the same rule

#### Scenario: He loses his place

- **WHEN** a player who had arrived loses his place for a season
- **THEN** he no longer reads as having arrived, and his dream is alive again

### Requirement: Anchors are seeded from the career he already has

When the world is built, each player's anchors SHALL be seeded from his recorded career: his summit from his recorded
seasons, and his dream from who he is and what he has already done. A player who appears later (an academy graduate, a
new generated player) SHALL be seeded when he first appears. A player with no recorded career SHALL get a summit from
his first seen run as a regular. Seeding SHALL NOT write a chapter dated in the past that the record does not support.

#### Scenario: A 28-year-old with a history

- **WHEN** the world is built with a 28-year-old who has played five seasons in a 9000 division and now plays in a 6000
  one
- **THEN** his summit reads 9000 from his record, and it has faded by however long ago that was

#### Scenario: A new academy graduate

- **WHEN** an academy graduate joins a senior squad
- **THEN** he has a dream within a week, and no summit until he has been a regular somewhere

### Requirement: A player's career is told in dated chapters

A player SHALL hold a bounded list of dated chapters: his dream forming, rising, falling or being let go, his arrival
and his losing it, a new summit, a first senior club, a step up or a fall, a homecoming, a reunion with a manager who
made him, and a first major honour. Chapters SHALL survive every move. When the list is full, the least significant
SHALL give way, and the dream's own turns and his summit SHALL outlast ordinary entries. Nothing in the simulation SHALL
decide anything by reading a chapter.

#### Scenario: A long career does not lose its best moments

- **WHEN** a 36-year-old with a long career has more turning points than the list holds
- **THEN** it still holds his summit, his arrival and his dream's last turn

#### Scenario: A chapter has a date

- **WHEN** a player arrives at his dream level
- **THEN** a chapter records the day, the club and the division
