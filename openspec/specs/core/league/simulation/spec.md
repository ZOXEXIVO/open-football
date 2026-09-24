# core/league/simulation Specification

## Purpose
Owns the matchday build/process pipeline internals: squad-selection importance, matchday squad backfill across a club's own sides, and non-matchday processing including season-end.

## Requirements

### Requirement: Squad selection importance drives rotation
For every fixture a club plays, the league simulation SHALL compute an importance score between 0.0 (dead rubber) and 1.0 (must-win) that determines how strong a lineup the club selects, based on what each side is playing for and, for cup ties, the knockout stage and opponent strength.

#### Scenario: Team with nothing left to play for
- **WHEN** a club's league position guarantees neither European qualification, promotion, nor relegation risk for the remainder of the season
- **THEN** the computed importance for its remaining fixtures drops significantly, allowing reserve and youth players to be selected

#### Scenario: Domestic cup final
- **WHEN** a fixture is a domestic cup final
- **THEN** the importance is clamped to a lower bound that keeps the match a strong-XI affair even under general congestion dampening

### Requirement: Matchday squad backfill from club depth
When a team cannot field a full matchday squad from its own registered players, the simulation SHALL backfill from the same club's other eligible teams (senior reserves and older youth sides) before a fixture is played, respecting availability (injury, suspension, international duty). A team SHALL NOT borrow any player whose own team has a fixture on the same calendar day, in any competition. This rule applies alike to senior call-ups, emergency goalkeeper and shortfall borrowing, youth sides' supplementary players, and overage development players. On a day when several of a club's sides have fixtures, only the most senior of them borrows from the club's other squads, in pathway order: first team, then Second/B/Reserve, then the youth sides from oldest to youngest. The club's other playing sides field only their own registered players. Together these rules ensure no player is named in two matchday squads on one day.

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

### Requirement: Season-end processing freezes standings and awards
When a league reaches its configured season-ending date, it SHALL record the champion (top table row), snapshot the season's awards before statistics are archived, freeze a final table (the annual aggregate for split-season leagues, otherwise the raw table), reset season-scoped dynamics, archive season statistics, and clear season-scoped disciplinary state (active suspensions, yellow-card accumulation, pending disciplinary cases).

#### Scenario: Standard (non-split) league reaches season end
- **WHEN** the current date matches the league's configured season-ending day/month
- **THEN** the champion is recorded from the current top table row, the final table snapshot equals the live table at that moment, and outstanding yellow-card accumulations are cleared going into the new season
