# core/match/engine/stub Specification

## Purpose
Defines what a stubbed match must produce when the `match-stub` feature stands in for the match engine: a believable
stat line for each player, without playing the match, so long simulations give clubs individual evidence to judge
players on.

## Requirements

### Requirement: A stubbed match gives every player who takes the pitch a full stat line

A stubbed match SHALL draw a stat line for every player who takes the pitch. The line SHALL cover shots, shots on
target, expected goals, passes attempted and completed, key passes, progressive passes and carries, dribbles attempted
and completed, crosses attempted and completed, passes into the box, tackles, interceptions, blocks, clearances,
fouls, cards and minutes. A goalkeeper's line SHALL also cover shots faced, saves, the chance value of the shots he
faced and goals prevented.

Volumes SHALL be in real-football units, with per-90-minute means set by his position group, and SHALL scale with the
minutes he played.

Each line SHALL agree with the match:

- a side's goals on its players' lines plus its own goals credited by the opponent SHALL equal its score
- a player's shots on target SHALL be at least his goals, and his shots at least his shots on target
- completed actions SHALL never exceed attempted ones
- a keeper's shots faced SHALL equal his saves plus the goals he conceded while on the pitch

#### Scenario: A position group's line looks like that position

- **WHEN** many stubbed matches are drawn between equal sides of ordinary ability
- **THEN** each position group's mean per-90 volumes for tackles, interceptions, shots, key passes, progressive passes
  and carries, dribbles and keeper shots faced sit within 15% of the real-football references the rating model is
  calibrated against

#### Scenario: A line agrees with the score

- **WHEN** any stubbed match finishes
- **THEN** every goal in the score is on exactly one player's line, every assist is on the line of a team-mate on the
  pitch at that moment, and no player's shots on target are fewer than his goals

#### Scenario: A cameo has a cameo's line

- **WHEN** a substitute plays the last ten minutes
- **THEN** his line's volumes are drawn for ten minutes rather than ninety

### Requirement: A stubbed performance follows the player

Each player's line SHALL be drawn from how he performs that day, read from:

- his ability against the opposition's ability
- his form on the day and his settledness, as the match already carries them
- his state of mind at kickoff, as the match already carries it
- for a fixture of high importance, his record in big matches

His position group SHALL set which volumes matter, and his relevant skills SHALL shift them: finishing moves his
conversion and shots on target, passing and vision move his completion and chance creation, tackling and positioning
move his defensive volume, and aggression and dirtiness move his fouls.

The draw SHALL be made where the performance is produced, in the stat line. Nothing SHALL be added to the finished
rating, so a good or bad day is visible in what he did.

The same seed SHALL draw the same match.

#### Scenario: A better player has a better line

- **WHEN** two players at the same position in the same fixture differ widely in ability, over many draws
- **THEN** the better player's mean line shows more of what his position does and a higher rating

#### Scenario: A bad day shows in the line

- **WHEN** one player's form on the day is well below his average
- **THEN** his line shows fewer completed actions and a lower rating, with no adjustment applied to the rating itself

#### Scenario: The same seed draws the same match

- **WHEN** a stubbed match is drawn twice from the same squads and seed
- **THEN** every player's line is identical

### Requirement: A player's season can run above or below his ability

Beyond the single day, each player SHALL have a season deviation: an amount by which his whole season runs above or
below his ability. It SHALL be drawn once per player per season, from his identity and the season, holding no state of
its own. Its average across players SHALL be zero. Its spread SHALL widen with youth and with low consistency,
concentration, professionalism and composure.

#### Scenario: Two equal players have different seasons

- **WHEN** two regulars of equal ability, position and club play a full stubbed season
- **THEN** their season ratings and output usually differ by more than match-to-match noise alone would produce

#### Scenario: A metronome swings less

- **WHEN** many seasons of a steady 28-year-old and of an erratic 19-year-old of equal ability are compared
- **THEN** the teenager's season ratings have the wider spread and neither's average sits above the other's by more
  than sampling noise

#### Scenario: A season holds together

- **WHEN** one player plays several matches in the same season
- **THEN** the same deviation applies to all of them, and a new season draws a new one

### Requirement: A stubbed rating comes from the stat line

A stubbed match SHALL rate each player with the engine's own rating model, reading his stat line and the team context.
The stub SHALL have no separate rating formula. Because its lines are already in real-football units, it SHALL NOT
apply the engine's volume conversion. A stubbed match SHALL also name its player of the match the way an engine match
does.

#### Scenario: Ratings land in the model's bands

- **WHEN** many stubbed matches are drawn between equal sides of ordinary ability
- **THEN** each position group's mean rating sits inside the season bands the rating model pins for that position

#### Scenario: A goal is worth what the model says

- **WHEN** a forward's line is compared with the same line and one more goal, in the same match context
- **THEN** the rating difference is exactly what the rating model gives for that line

### Requirement: A stubbed match produces cards and injuries

A stubbed match SHALL draw fouls and, from them, yellow and red cards, at match rates in line with the engine's. A
player sent off SHALL leave the pitch at that moment and credit nothing after it. A stubbed match SHALL also draw
injuries, with a chance that rises with low condition and with his injury proneness, and hand the injury to the player
the way an engine match does. An injured player SHALL be replaced when a substitute is available. Bans and time out
SHALL follow from the normal post-match handling, with nothing stub-specific downstream.

#### Scenario: A dirty player collects more cards

- **WHEN** two players of the same position differ widely in aggression and dirtiness, over many draws
- **THEN** the dirtier player commits more fouls and collects more cards

#### Scenario: A red card ends the stint

- **WHEN** a player is drawn a red card at some minute
- **THEN** his minutes stop at that minute, and no later goal or assist is credited to him

#### Scenario: An injury reaches the player

- **WHEN** a player is drawn an injury that persists past the match
- **THEN** the persisted player carries that injury through the same post-match path an engine match uses

### Requirement: A stubbed world keeps its results in band and its cost low

Across a stubbed world over a season, results SHALL stay inside the bands the stub is calibrated to: goals per match,
share of draws, share of goalless draws, and home share of goals. A stubbed world day SHALL NOT cost more than 15% over
the previous stub. With the `match-stub` feature off, nothing in match simulation SHALL change.

#### Scenario: A stubbed season reads like football

- **WHEN** a stubbed world plays a full season
- **THEN** it reads between 2.6 and 2.9 goals per match, 22% to 28% draws, 5% to 8% goalless draws and 53% to 58% home
  goals

#### Scenario: The real engine is untouched

- **WHEN** the `match-stub` feature is off
- **THEN** a seeded engine match produces exactly what it produced before this change
