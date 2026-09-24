# core/match/engine/tactics Specification

## Purpose
Owns each player's target position on the pitch — the base tactical position/role table and how it is continuously adjusted by team shape, ball location, and phase of play.

## Requirements

### Requirement: Player off-ball movement and positioning
Each player's target position SHALL be continuously influenced by their assigned tactical position/role, the team's current shape, the location of the ball, and the phase of play (attacking, defending, transition), rather than by a fixed, static formation slot.

#### Scenario: Team shape compresses toward the ball
- **WHEN** the team is out of possession and the ball is in a wide area
- **THEN** outfield players shift their positioning toward that side rather than holding their nominal formation coordinates unchanged

#### Scenario: Forward creates space off the ball
- **WHEN** an attacking player without the ball is being tightly marked in a congested area
- **THEN** that player's movement can seek open space away from their marker rather than remaining static
