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

### Requirement: One reach rule decides who can head the ball, and a won aerial contest is always played
Whether a player can head the ball SHALL be decided by one heading reach rule. The ball is headable when it is:

- within the player's heading reach, 0.75 m measured across the grass and the same for every role;
- above the highest ball a boot can strike;
- at or below the player's own jumping ceiling.

A head SHALL NOT reach as far across the grass as a boot does.

That one rule SHALL decide all of the following:

- when a delivery awarded by an aerial contest has reached its winner;
- whether the winner can head it;
- every header in open play, whether or not a contest awarded the ball.

On arrival the winner SHALL play the ball while it is still headable. The role decides whether that is a header at
goal, a knock-down or a clearance. The award SHALL NOT lapse because the winner was judged out of reach by a different
rule than the one that judged the delivery arrived.

The award reserves the ball for the winner only while the winner can head it. If the ball falls to boot height first,
the award SHALL be released at that height, and the ball SHALL be free for any player to claim. No height SHALL leave
the ball reserved for a winner who may no longer head it.

A contest decided while the ball is already descending in flight SHALL leave the ball on its own flight to where it
was aimed. The ball SHALL NOT be relaunched or re-aimed at the winner's head to make the header possible.

While a won delivery waits at its aim point for a winner who is not yet within reach, the ball SHALL be held in the
heading band. Its descent and its speed across the grass SHALL be slowed along its own line, and it SHALL NOT be turned
toward goal or toward the winner. This SHALL hold whatever the delivery's outcome: a header, a clearance or a ball
hooked behind. The wait SHALL end when the winner gets within reach, when the ball comes down to boot height, when the
delivery's deadline passes, or when another player touches the ball.

#### Scenario: A won cross is headed when it arrives
- **WHEN** an attacker wins the aerial contest for a cross, and the ball reaches them within striking distance above
  boot height
- **THEN** the attacker heads it at their next decision, at goal, as a knock-down or clear, as their role decides
- **AND** the award does not lapse with the ball unplayed

#### Scenario: Reach does not depend on the role
- **WHEN** a ball at head height is 0.6 m across the grass from a forward, from a midfielder and from a defender, each
  inside their heading reach and under their jumping ceiling
- **THEN** each of them can head it

#### Scenario: A head does not reach as far as a boot
- **WHEN** a ball at head height is 1.5 m across the grass from a player, a distance at which a ball at his feet would
  be his to kick
- **THEN** he cannot head it

#### Scenario: Height counts for nothing in the distance
- **WHEN** a ball 2.4 m up is directly above a player whose jumping ceiling is higher than that
- **THEN** the player can head it, because the ball is no distance away across the grass

#### Scenario: A ball that drops to boot height is free
- **WHEN** a ball awarded to a contest winner falls to boot height before the winner has played it
- **THEN** the award is released at that height and any player may claim the ball, the winner included

#### Scenario: A ball above the winner's leap waits for the winner
- **WHEN** an awarded ball is still above the winner's jumping ceiling as it comes over them
- **THEN** no header is made until it comes down into reach, and nobody else may take the ball from the winner in
  the meantime

#### Scenario: A cross already on its way is not relaunched
- **WHEN** the aerial contest for an open-play cross is decided while the ball is already descending toward the box
- **THEN** the ball carries on along that flight to where it was going, and is not relaunched toward the winner

#### Scenario: A won ball waits in the band for its winner
- **WHEN** a won delivery comes down into the heading band at its aim point while its winner is still out of reach
  and closing
- **THEN** the ball stays above boot height while he closes, within the delivery's deadline
- **AND** when he gets within reach he plays it, as his role decides

#### Scenario: The hold does not turn the ball
- **WHEN** a won delivery is held while its winner closes
- **THEN** the ball keeps the direction across the grass it was travelling in, and only slows

#### Scenario: A winner who never arrives leaves the ball to anybody
- **WHEN** the winner of a held delivery does not get within reach before its deadline
- **THEN** the hold ends at the deadline, the award with it, and any player may claim the ball
