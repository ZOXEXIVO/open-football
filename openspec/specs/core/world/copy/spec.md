# core/world/copy Specification

## Purpose
Defines what copying a simulated world guarantees. A copy is equivalent to, and independent of, the world it was taken from, and costs what the daily tick changes rather than the history the world has accumulated.

## Requirements

### Requirement: A copy is equivalent to its original
A copy of a world SHALL hold the same state as the world it was taken from. It SHALL digest identically over every part the world fingerprint covers, and reading any record from it SHALL give the same values, in the same order, as reading that record from the original.

#### Scenario: Copying a world that has simulated days
- **WHEN** a world that has simulated several days is copied
- **THEN** the copy's fingerprint SHALL equal the original's

#### Scenario: Reading history from a copy
- **WHEN** a team's match history, a player's season statistics or a country's retired players are read from a copy
- **THEN** they SHALL list the same records in the same order as the original

### Requirement: A copy and its original are independent
A change made to either a world or its copy SHALL NOT be observable in the other. That includes simulating days, appending to history the two share, editing a record, and releasing either one.

#### Scenario: Simulating the copy forward
- **WHEN** a copy is simulated forward by a day
- **THEN** the original's fingerprint SHALL be unchanged

#### Scenario: Appending to shared history
- **WHEN** a match is recorded in a copy whose match history is still shared with the original
- **THEN** the original's match history SHALL be unchanged, and the copy's SHALL end with the new match

#### Scenario: Releasing the copy
- **WHEN** a copy is released
- **THEN** the original SHALL stay intact and digest as it did before

### Requirement: A copy shares the history the simulation only appends to
Copying a world SHALL share, rather than duplicate, every collection of records that the simulation only ever appends to and whose copy cost is measurable. Examples are archived matches, finished-season statistics, decisions taken and retired players. A copy SHALL duplicate only the part of each such collection that is still open for appending. A record appended after the copy SHALL exist only on the side that appended it. State the daily tick rewrites SHALL still be copied in full.

#### Scenario: Copying after a simulated season
- **WHEN** a world that has simulated a full season is copied
- **THEN** the sealed records of every shared collection SHALL be the same stored records in the copy and the original, not duplicates
- **AND** only each collection's open part SHALL be duplicated

#### Scenario: History no longer drives copy cost
- **WHEN** the per-collection copy census runs on the shipped world after a simulated season
- **THEN** no collection the simulation only appends to SHALL account for 1% or more of the world copy's time
