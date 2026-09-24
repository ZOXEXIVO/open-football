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
Each team SHALL be limited to a bounded number of discrete substitution stoppages and a bounded number of total player changes per match, and a second (or further) change made during a stoppage a team-mate has already interrupted SHALL not consume an additional stoppage.

#### Scenario: Multiple changes bundled into one stoppage
- **WHEN** a team brings on more than one substitute during the same dead-ball stoppage
- **THEN** only one substitution window is counted as spent, not one per player change

### Requirement: Injury substitution resolution
An injured player SHALL be substituted at the next periodic medical check when their team still has a substitution available, or, if no substitution is available, SHALL return to play in a diminished state once a bounded recovery period has elapsed.

#### Scenario: Injured player with a substitute available
- **WHEN** a player is marked injured and their team still has a substitution available at the next medical check
- **THEN** the team can replace the injured player with a substitute

#### Scenario: Injured player with no substitutes remaining
- **WHEN** a player is marked injured and their team has no substitutions left
- **THEN** the player remains out of active play until the treatment period elapses and then resumes playing in their role's default state
