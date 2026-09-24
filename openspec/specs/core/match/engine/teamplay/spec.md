# core/match/engine/teamplay Specification

## Purpose
Owns team-level tactical organization — coach instructions, defensive/attacking shape, and how the team as a whole tracks and marks opposing attackers.

## Requirements

### Requirement: Marking and defensive shape
Defending players SHALL track and mark opposing attackers based on the game state (zonal or player-oriented marking implied by role and phase), and a marked attacker SHALL have a reduced but not eliminated chance of shaking off close marking through evasive movement.

#### Scenario: Tightly marked attacker attempts evasion
- **WHEN** an attacking player is closely marked and attempts to create separation
- **THEN** the evasion attempt's success is influenced by both the attacker's and marker's relevant attributes rather than always succeeding or always failing
