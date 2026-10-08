# core/club/player/condition Specification

## Purpose
Defines the player's own record of recent football: competitive and friendly minute windows, the football he has
absorbed over about a season and the match exposure development reads from it, and what a day of off-season camp does
to his condition.

## Requirements

### Requirement: Friendly minutes have their own window

A match's minutes SHALL be recorded in the competitive 7-day and 30-day windows only when the match is competitive.
Friendlies, youth-league games and reserve-league games SHALL be recorded only in a separate recency-weighted 30-day
friendly window, which decays on the same schedule. Rotation and selection SHALL keep reading the competitive windows
only.

#### Scenario: A youth-league start
- **WHEN** a player plays 90 minutes in a friendly-flagged youth-league game
- **THEN** his friendly 30-day window rises by 90, and his competitive 7-day and 30-day windows are unchanged

### Requirement: Match exposure measures recent football against a regular's

A player's record SHALL keep a stock of the football he has absorbed:
- every match SHALL add his minutes to it: competitive minutes in full, and friendly, youth-league and reserve-league
  minutes at half;
- the stock SHALL fade every day, with a time constant of one season (365 days);
- when a world is built, the stock SHALL be seeded from the player's two most recent recorded seasons, faded as if he
  had played them. A player with no recorded seasons SHALL be seeded from his squad role, the way a regular, a squad
  player or a youth player would hold it. A player created during play SHALL start with an empty stock.

A player's match exposure SHALL be computed as follows:
- divide the stock by a regular's: what a player who started 30 league matches in each of the two previous seasons
  holds at the end of the summer break
- clamp the result to 0..1

Match exposure SHALL NOT read the competitive 7-day and 30-day windows or the friendly 30-day window. Those windows
SHALL keep serving rotation, selection and the development tick's rate.

#### Scenario: A weekly starter
- **WHEN** a player has started a 90-minute match every week for two seasons
- **THEN** his match exposure is 1.0 at every point of the season, the end of the summer break included

#### Scenario: A player who has not played
- **WHEN** a player has no recorded minutes
- **THEN** his match exposure is 0.0

#### Scenario: A month on the bench
- **WHEN** a player who has started every week for two seasons plays no minutes for 30 days
- **THEN** his match exposure stays above 0.9

#### Scenario: Ten starts after years of watching
- **WHEN** a player with no minutes in the past three seasons starts ten 90-minute competitive matches
- **THEN** his match exposure is below 0.5

#### Scenario: A regular on the world's first day
- **WHEN** a world is built containing a player with 30 or more league appearances in each of his two most recent
  recorded seasons
- **THEN** his match exposure on the first day is 1.0

#### Scenario: A youth-league regular
- **WHEN** a player starts every week in a friendly-flagged youth league for a season
- **THEN** his match exposure settles near half of a weekly senior starter's

### Requirement: An off-season camp day rebuilds sharpness only

On each off-season camp day, a non-injured player's match readiness SHALL rise by `0.3 + 0.4 × facility quality`, up to
20. A camp day SHALL NOT change any technical, mental, physical or goalkeeping attribute. An injured player SHALL be left
unchanged.

#### Scenario: A summer of camps
- **WHEN** a player spends a full off-season in camps
- **THEN** his match readiness rises, and his attributes change only through the development tick and the club's
  training sessions
