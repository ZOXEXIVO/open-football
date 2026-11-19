# core/match/engine/substitution Specification

## Purpose
Owns when and how a team makes a substitution — the pressure that drives the decision, the bounded windows and change limits, and resolving an injured player at the next medical check.

## Requirements

### Requirement: Substitution timing (bench pressure)
The likelihood that a team makes a substitution at a given moment SHALL be driven by a continuous combination of factors — time remaining, goal difference (chasing a deficit or protecting a lead), the fitness of the most fatigued starters, any acute trouble (a card, a costly error, a collapsing individual performance), and the responsible coach's own tendencies — rather than by fixed substitution windows tied to specific match minutes.

#### Scenario: Trailing team substitutes earlier than a level team
- **WHEN** two otherwise-identical matches differ only in that one team is behind on the scoreline in the second half
- **THEN** the trailing team's first substitution becomes available earlier in the match than the level team's

#### Scenario: Half-time substitutions are effectively free
- **WHEN** a team wants to make a change at half time
- **THEN** that change requires less accumulated pressure to occur than the same change would during open play

### Requirement: Substitution windows and change limits
Each team SHALL be limited to a bounded number of discrete substitution stoppages and a bounded number of total player changes per match, and a second (or further) change made during a stoppage a team-mate has already interrupted SHALL not consume an additional stoppage. When a match goes to extra time each team SHALL receive one additional change and one additional stoppage, and a change made at the extra-time interval SHALL not consume a stoppage.

#### Scenario: Multiple changes bundled into one stoppage
- **WHEN** a team brings on more than one substitute during the same dead-ball stoppage
- **THEN** only one substitution window is counted as spent, not one per player change

#### Scenario: Extra time grants a usable extra change
- **WHEN** a team has used all its stoppages and changes in normal time and the match goes to extra time
- **THEN** the team can still make one further change in extra time

### Requirement: Injury substitution resolution
An injured player SHALL be resolved by the severity decided in the match: a serious injury SHALL be substituted at the next dead ball when the team has a change available and otherwise SHALL leave the pitch, leaving the team short-handed; a hurt player SHALL be substituted when the bench pressure and the player's diminished state warrant it and otherwise SHALL continue diminished; a knock SHALL not force a change. Exhaustion (condition below the critical threshold without an injury) SHALL be recorded as a fatigue change, not an injury. A half-time or interval rest SHALL NOT clear an injury. A player who has been sent off SHALL never be selected for a forced change.

#### Scenario: Injured player with a substitute available
- **WHEN** a player sustains a serious injury and their team still has a change available at the next dead ball
- **THEN** the team replaces the injured player with a substitute, recorded with an injury reason

#### Scenario: Injured player with no substitutes remaining
- **WHEN** a player sustains a serious injury and their team has no changes left
- **THEN** the player leaves the pitch and the team finishes with one fewer player, while a hurt player in the same situation returns to play diminished once treatment is complete

#### Scenario: Hurt player continues diminished
- **WHEN** a player sustains a hurt-severity injury and the bench pressure does not warrant a change
- **THEN** the player returns to play in their role with reduced effective ability for the remainder of the match

#### Scenario: Exhausted player is not an injury
- **WHEN** an uninjured player's condition falls below the critical threshold and they are replaced
- **THEN** the change is recorded with a fatigue reason and no injury is recorded for the player after the match

### Requirement: The bench reads the coach's match
What the bench is asked for (attackers, defenders, a midfielder or fresh legs) SHALL come from two things: the coach's commitment reading, and the coach's read of the run of play that the side on the pitch is playing to. The bench SHALL see the real score from the same moment the pitch does. The bench's timing SHALL stay within its calibration:
- about four and a half changes a team;
- a first change around the hour, with half-time a minority of first changes;
- no player taken off at the tick the player came on, and no discretionary change of a player who has played under 15 minutes.

#### Scenario: A side chasing the match brings on attackers
- **WHEN** a side's commitment reading is chasing the match and it makes a discretionary change
- **THEN** the bench is asked for an attacking player

#### Scenario: The bench sees the score with the pitch
- **WHEN** a side goes two goals behind in the 20th minute
- **THEN** its bench reads the deficit from that moment, as its players on the pitch do

#### Scenario: The bench keeps its rhythm
- **WHEN** 400 equal-strength matches are played with benches
- **THEN** each team makes 4.0 to 5.0 changes on average, and the first change comes on average between the 55th and the 66th minute
- **AND** no player is taken off at the tick the player came on, and no discretionary change takes off a player who has played under 15 minutes
