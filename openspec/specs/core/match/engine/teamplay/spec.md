# core/match/engine/teamplay Specification

## Purpose
Owns team-level tactical organization — coach instructions, defensive/attacking shape, and how the team as a whole tracks and marks opposing attackers.

## Requirements

### Requirement: Marking and defensive shape
Defending players SHALL track and mark opposing attackers based on the game state (zonal or player-oriented marking implied by role and phase), and a marked attacker SHALL have a reduced but not eliminated chance of shaking off close marking through evasive movement.

#### Scenario: Tightly marked attacker attempts evasion
- **WHEN** an attacking player is closely marked and attempts to create separation
- **THEN** the evasion attempt's success is influenced by both the attacker's and marker's relevant attributes rather than always succeeding or always failing

### Requirement: The closer stands across the line of the carrier's delivery
A defending player closing an opposing ball carrier SHALL steer for a position a stride off the carrier, goal-side of him, on the line from the carrier toward the danger in the defending side's box: its goal and the carrier's team-mates inside the box.

He SHALL lean from that line toward the carrier's line of run by an amount that:
- rises continuously with the grass the carrier has ahead of him along his run;
- rises with the carrier's pace over his own;
- falls to nothing when the carrier has no grass left.

Pace SHALL enter only as the carrier against the closer, so the position does not change with the level of football when the two are equally matched.

The same rule SHALL apply on every part of the pitch and to every player who closes the carrier, whatever his role or position. A committed challenge still goes to the ball.

#### Scenario: A winger at the byline is closed from the inside
- **WHEN** a carrier in a wide area near the defending side's goal line has team-mates in the box and no grass left toward the goal line
- **THEN** the man closing him stands between him and the box, on the line a cross or cutback would take, rather than between him and the goal line

#### Scenario: A delivery from there meets the closer
- **WHEN** that carrier delivers into the box past the closer who has taken the line
- **THEN** the closer is on the delivery's line, the crosser prices him, and the delivery can be blocked by him

#### Scenario: A quick winger with the run on is not simply waved through
- **WHEN** a wide carrier with open grass toward the goal line is quicker than the man closing him
- **THEN** the closer stands nearer the carrier's line of run than he would against a slower carrier in the same position

#### Scenario: A carrier through the middle is closed goal-side
- **WHEN** a carrier runs straight at goal through the middle, with his team-mates in the box ahead of him
- **THEN** the closer stands goal-side of him on his line toward goal, with no lateral shift toward either flank

#### Scenario: Every closer takes the same position
- **WHEN** a full-back, a wide midfielder, a centre-back or a forward of the same pace each close the same carrier
- **THEN** each steers for the same position

#### Scenario: The position does not walk with the level
- **WHEN** an equally matched wide carrier and closer meet in the same situation at a low and at a high level of football
- **THEN** the closer steers for the same position at both levels

### Requirement: Short-handed team shape
When a team is reduced below eleven players by a sending-off or an injury with no change available, the team SHALL re-anchor its shape to a formation for the remaining number of players chosen by the coach's situational logic (sacrificing the role the situation can most afford), SHALL keep that shape across the half-time and extra-time ends swap, and the coach's in-match evaluation SHALL read the player-count deficit as an input to instruction and formation choices.

#### Scenario: Red card reshapes the side
- **WHEN** a side's forward is sent off in the 30th minute while the match is level
- **THEN** the remaining ten re-anchor to a ten-player formation, the vacated role is not left empty, and the same ten-player shape is in force after half time

#### Scenario: Coach reads the deficit
- **WHEN** a side is a player down and leading
- **THEN** the coach's instruction evaluation treats the deficit as pressure to protect rather than push, independent of the minute gate that governs score reactions

### Requirement: Offside-trap step-up
A back line holding its line SHALL be able to step up together to leave an attacker offside at the moment a pass is about to be played, with the decision driven by the line's organisation and anticipation, the keeper's organising voice and the pairing chemistry, and with a failure mode in which a late or uncoordinated step-up plays the attacker onside with a clear run.

#### Scenario: Coordinated line springs the trap
- **WHEN** a well-organised back line with high chemistry reads an imminent through ball to an attacker on the shoulder of the last defender
- **THEN** the line steps up together before the ball is played and the attacker is left in an offside position

#### Scenario: Poorly organised line is caught out
- **WHEN** a disorganised back line attempts the same step-up and one defender is late
- **THEN** the attacker is played onside behind the line and the attempt is recorded as a failed trap
