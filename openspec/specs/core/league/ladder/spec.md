# core/league/ladder Specification

## Purpose
Describes how a country's divisions stack into a ladder of rungs, which league each one relegates into, and how many
sides cross every boundary, so the season-end swap and the league tables' zone markers read the same counts.

## Requirements

### Requirement: A country's divisions form a ladder of rungs
The system SHALL read a country's leagues as a ladder whose rungs are (tier, group level) pairs. The rung directly below
a league SHALL be the next level of its own tier when any league sits on that rung, and level 0 of the next tier
otherwise. A league SHALL relegate only into leagues with promotion places on the rung directly below it. A tier-0
league, or a league with no relegation places, SHALL relegate nobody.

#### Scenario: A division above ranked groups
- **WHEN** a tier-2 division sits above a tier-3 Gold group (level 0) and Silver group (level 1)
- **THEN** it relegates into Gold only, Gold relegates into Silver, and Silver relegates into the tier-4 leagues

#### Scenario: A league with no relegation places
- **WHEN** a league declares zero relegation places
- **THEN** no side drops out of it, whatever sits below it

### Requirement: A division alone on its rung feeds every group below it, champions-first
A league SHALL relegate into every group on the rung below it when both of these hold:

- it has no parallel zone of its own competition on its rung
- every promoting league on the rung below is a group of one competition, and there are at least two such groups

Its relegation places SHALL then be dealt champions-first: one to each group in league-id order, then one more to each
group that still has promotion places, and so on, until no relegation places remain. No group SHALL receive more
places than its own promotion places.

Every other boundary SHALL have a single lower partner and SHALL move the smaller of the upper league's relegation
places and the partner's promotion places. When the relegating rung and the rung below are both split into groups,
zone k (in id order) SHALL pair with group k (in id order).

#### Scenario: Four places over two groups
- **WHEN** a division relegating four sits above two groups that promote two each
- **THEN** each group is allotted two places

#### Scenario: Fewer places than the groups could promote
- **WHEN** the same division relegates three, or one
- **THEN** with three, the first group is allotted two and the second one; with one, the first group is allotted one
  and the second none

#### Scenario: A ranked group feeds regional zones
- **WHEN** a Silver group relegating four sits above four regional zones of one competition that promote one each
- **THEN** each zone is allotted one place

### Requirement: Both sides of a boundary report the same crossing counts
A league's promoted count SHALL equal the places the league or leagues on the rung above allot to it. Its relegated
count SHALL equal the sum of the places it allots across the rung below. A split-season league SHALL report no
relegation from a tournament table; its relegation is read from the annual aggregate.

#### Scenario: Group allotment shown on the group's own table
- **WHEN** a division relegates three over two groups promoting two each
- **THEN** the first group reports two promoted, the second reports one, and the division reports three relegated

#### Scenario: A group nobody relegates into
- **WHEN** a league declares promotion places but no league on the rung above allots it any
- **THEN** it reports zero promoted
