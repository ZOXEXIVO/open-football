# core/match/engine/ball Specification

## Purpose
Owns the ball itself — flight physics, boundary/goal detection, possession claims, contested duels for a loose or in-flight ball, and offside evaluation on passes.

## Requirements

### Requirement: Goal detection and scoring
The system SHALL detect when the ball fully crosses the goal line within the goal frame and SHALL credit the goal to the scoring team's tally and to the responsible player (including recording own goals against the scoring team when the ball is turned into a player's own net), triggering the post-goal restart sequence.

#### Scenario: Own goal credited correctly
- **WHEN** a defending player deflects the ball into their own goal
- **THEN** the goal is added to the opposing team's tally and recorded as an own goal against the defending player, not as a goal for any attacking player

### Requirement: Pass interception contest
A pass in flight SHALL be contestable by defending players positioned to intercept it, evaluated once per opposing player at that player's closest point of approach to the ball's flight path, rather than being resolved once at the moment the ball is kicked.

#### Scenario: Defender positioned on the passing lane
- **WHEN** an opposing player's closest approach to the flight path of an in-progress pass falls within interception range
- **THEN** that player has a chance to win the ball determined by the contest at that closest-approach moment, not by a single check at the pass's origin

### Requirement: Ball possession claims and loose balls
The system SHALL track who currently owns the ball and how they came by it (received a pass, won a loose ball, intercepted, or tackled it away), SHALL resolve claims on an uncontrolled or contested ball by proximity and challenge outcome, and SHALL treat a ball received far outside close control range as not yet under control rather than as an automatically completed reception.

#### Scenario: Ball received well beyond control range stays loose
- **WHEN** a pass arrives near a team-mate but beyond the distance at which the ball is considered under a player's control
- **THEN** the ball is not credited as securely possessed by that player and remains contestable

#### Scenario: Possession source is attributed correctly
- **WHEN** a player gains the ball by winning a tackle rather than by receiving a pass
- **THEN** the resulting possession is attributed to a tackle-win rather than to a pass reception

### Requirement: Offside detection on passes
The system SHALL evaluate whether a pass's intended receiver was in an offside position at the moment the ball was played, using the position of the ball, the receiver, and the second-last defender on the receiving team, and SHALL apply a small tolerance to absorb positional ambiguity rather than flagging on exact coordinate equality. The passer's own decision-making SHALL account for the same offside line when choosing whether to attempt the pass.

#### Scenario: Receiver level with the second-last defender is not offside
- **WHEN** the receiver's position at the moment of the pass is within the tolerance band of the second-last defender's line
- **THEN** the receiver is not flagged offside

#### Scenario: Receiver clearly beyond the defensive line is offside
- **WHEN** the receiver is positioned significantly beyond the second-last defender toward the opponent's goal at the moment the ball is played
- **THEN** the pass is flagged offside once the receiver becomes involved in the passage of play
