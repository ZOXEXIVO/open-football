# core/club/team/tactics Specification

## Purpose
Owns the system a club's side plays: which formation its head coach chooses, the moments at which the coach chooses it again, and the formation a match is played in.

## Requirements

### Requirement: The coach chooses the system on taking charge and when the squad changes
A side's formation SHALL be chosen by its head coach from two things: the coach's preferences, and the players fit to play. It SHALL be chosen when the world is created, when a new head coach takes charge, and when a transfer window closes. It SHALL NOT change at any other time.

#### Scenario: A new coach brings a system
- **WHEN** a new head coach takes charge of a side
- **THEN** the side's formation is chosen again from the new coach's preferences and the players fit to play

#### Scenario: A rebuilt squad
- **WHEN** a transfer window closes after a side has signed and sold players
- **THEN** the side's formation is chosen again from the squad as it now stands

#### Scenario: Nothing changes between those moments
- **WHEN** no new coach takes charge and no window closes between two matches
- **THEN** the side starts both matches in the same formation

### Requirement: A match is played in the side's own system
In every competitive match a side SHALL start in its own formation. It SHALL answer the opponent through its game plan. It SHALL NOT swap to a shape keyed on the opponent's formation. No single coaching attribute threshold SHALL switch the answer to the opponent on or off.

#### Scenario: The opponent's shape does not pick ours
- **WHEN** a side whose formation is 4-4-2 plays an opponent in 4-3-3
- **THEN** it starts the match in 4-4-2

#### Scenario: No threshold
- **WHEN** two coaches whose tactical knowledge is 13 and 14 manage the same side against the same opponent
- **THEN** both start in the side's own formation, and how far each plan answers the opponent differs only by the continuous difference between the two coaches
