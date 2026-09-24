# core/match/engine/officiating Specification

## Purpose
Owns the referee: restart set-up, foul and disciplinary decisions, advantage play, and the composed home-advantage effect applied through officiating bias.

## Requirements

### Requirement: Kickoff and restart set-up
At the start of each period and after every goal, the system SHALL position all players according to a restart shape before play resumes: the team taking the restart has a designated taker and, when a team-mate is available, a nearby partner to receive the first pass, while the opposing team is kept a minimum distance from the ball.
Other dead-ball restarts (throw-ins, corners, goal kicks, free kicks) SHALL likewise position the relevant players before the ball is put back into play.

#### Scenario: Kick-off has a receiving option
- **WHEN** a team with at least two outfield players available takes a kickoff
- **THEN** the taker is not left isolated — a team-mate is stationed nearby as a pass option and the opposing team is kept outside the required restraining distance

#### Scenario: Kick-off with only a goalkeeper outfield
- **WHEN** the kicking side has no available outfield team-mate besides the taker
- **THEN** no partner station is assigned and the taker proceeds without one

### Requirement: Foul-call probability on tackle contact
The referee SHALL assess contact during a tackle for a foul, using the severity of the contact, and SHALL escalate the resulting restart to a penalty when the foul occurs inside the defending team's penalty area.

#### Scenario: Reckless contact raises foul-call probability
- **WHEN** a tackle attempt is classified as reckless-severity contact rather than normal contact
- **THEN** the probability the referee calls a foul on that contact is higher than for normal-severity contact

#### Scenario: Contact inside the penalty box can be a penalty
- **WHEN** foul contact against an attacking player occurs inside the defending team's penalty area and the referee's whistle goes
- **THEN** the resulting restart is a penalty kick rather than a free kick

### Requirement: Officiating — foul detection and disciplinary action
The referee SHALL assess contact during duels for a foul, using the severity of the contact, the referee's own strictness/leniency profile, and situational modifiers (location on the pitch, match temperature, and a bounded home-team bias scaled by crowd intensity) to decide whether to stop play, and SHALL be able to issue yellow or red cards for qualifying conduct.

#### Scenario: Match temperature raises foul-call sensitivity
- **WHEN** the match has recently accumulated fouls, cards, or heated incidents (elevated match temperature)
- **THEN** subsequent borderline contact is more likely to be called a foul than the same contact earlier in a calm match

#### Scenario: Two yellow cards produce a sending-off
- **WHEN** a player receives a second yellow card in the same match
- **THEN** that player is sent off and recorded as a red card

### Requirement: Advantage and set-piece restarts
The referee SHALL be able to play advantage rather than immediately stopping play for a foul, and every stoppage (foul, offside, ball out of play) SHALL resume via the appropriate restart type (free kick, throw-in, corner, goal kick, penalty) with players positioned accordingly.

#### Scenario: Advantage played after a foul
- **WHEN** the fouled team retains a promising attacking position immediately after the contact
- **THEN** the referee can delay the whistle to let play continue before resolving the foul call

### Requirement: Home advantage as a composed effect
Home advantage SHALL be expressed through multiple distinct channels (e.g. crowd-driven referee bias, environmental/psychological modifiers) that can each be independently isolated or disabled for testing, rather than as a single undifferentiated home bonus.

#### Scenario: Disabling one home-advantage channel does not remove the others
- **WHEN** the referee's crowd-driven bias channel is held flat for test purposes
- **THEN** other home-advantage channels in the match continue to operate independently
