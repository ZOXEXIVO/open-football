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
The system SHALL snapshot every attacker's offside status at the moment a team-mate plays the ball, using the position of the ball, each attacker, and the second-last defender on the defending team, and SHALL apply a small tolerance to absorb positional ambiguity rather than flagging on exact coordinate equality. The snapshot SHALL remain valid until the ball is next deliberately played by any player — not for a fixed time — and the offence SHALL be committed by whichever attacker first becomes involved in play from an offside position, whether or not they were the intended receiver, including a ball that reaches them from a defender's deflection, a keeper's save or the woodwork. A deliberate play by a defender SHALL reset the snapshot. The offence SHALL restart with an indirect free kick to the defending side, and the passer's own decision-making SHALL account for the same offside line when choosing whether to attempt the pass.

#### Scenario: Receiver level with the second-last defender is not offside
- **WHEN** the receiver's position at the moment of the pass is within the tolerance band of the second-last defender's line
- **THEN** the receiver is not flagged offside

#### Scenario: Receiver clearly beyond the defensive line is offside
- **WHEN** the receiver is positioned significantly beyond the second-last defender toward the opponent's goal at the moment the ball is played
- **THEN** the pass is flagged offside once the receiver becomes involved in the passage of play, and play restarts with an indirect free kick

#### Scenario: Unintended attacker plays the ball from offside
- **WHEN** a pass intended for an onside team-mate is instead first played by a different attacker who was in an offside position when the ball was struck
- **THEN** that attacker is flagged offside

#### Scenario: Long ball in the air for longer than the old expiry
- **WHEN** a pass is in flight for more than three seconds and is then first played by an attacker who was in an offside position when it was struck
- **THEN** the attacker is flagged offside

#### Scenario: Rebound off the keeper to an offside attacker
- **WHEN** a shot is saved and the rebound is first played by an attacker who was in an offside position when the shot was struck
- **THEN** that attacker is flagged offside

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

### Requirement: A pass in flight can be blocked by the men it passes
Every opponent a pass, cross included, passes on its way SHALL have one chance per flight to block it. The chance SHALL be one rule that changes continuously with the blocker's time to react, the time the ball takes to reach him:

- With almost no time, the block is a **charge-down**. It is decided by his body: how near the ball's line he stands, and whether the ball is below his standing reach when it gets to him. It SHALL NOT depend on where on the pitch the pass is played.
- With time to read the pass, the block is a **lunge**. It is decided by how well he reads it and how far he must stretch, below the height of a leg. It SHALL apply only toward his own goal, rising as the ball gets nearer to it.

Skill SHALL enter as a duel between the blocker and the striker's technique for that strike: crossing for a cross, passing otherwise. A blocker and a striker of equal standing SHALL produce the same block rate at every level of football.

A block SHALL turn the ball loose, not hand it to the blocker, except when it was struck into his feet and he controls it. The block SHALL count as his touch.

#### Scenario: A ball struck into a man a stride away is charged down
- **WHEN** a pass is struck along a line through an opponent a stride from the striker, and the ball is below his standing reach as it reaches him
- **THEN** he has a chance of blocking it that does not depend on having read the pass, and it is greatest when he is square on the line

#### Scenario: The charge-down applies anywhere on the pitch
- **WHEN** the same pass, struck into a man a stride away, is played once on the halfway line and once in front of the blocker's own goal
- **THEN** the charge-down chance is the same in both places

#### Scenario: A ball over him is not charged down
- **WHEN** a lofted delivery is already above an opponent's standing reach as it passes him a stride from the striker
- **THEN** he cannot block it

#### Scenario: No lunge far from his own goal
- **WHEN** a pass passes within leg's reach of an opponent who had time to read it, in the half his team attacks
- **THEN** he does not lunge at it, and only the charge-down, which fades with his time to react, can block it

#### Scenario: One chance per man per flight
- **WHEN** a pass passes the same opponent on one flight
- **THEN** that opponent has at most one chance to block it on that flight

#### Scenario: The rate does not walk with the level
- **WHEN** blockers and strikers of equal standing meet in the same situations at a low and at a high level of football
- **THEN** the share of passes blocked is the same at both levels

### Requirement: A block's deflection follows the blocker's time to react
Where a blocked ball goes SHALL follow the same time to react that decides the block. The less time the blocker had, the more of the ball's own line and pace the deflection SHALL keep. The more time he had, the more it SHALL be directed away from his own goal. A blocked ball that crosses the blocker's own goal line wide of the posts SHALL restart with a corner.

#### Scenario: A cross charged down near the goal line goes behind
- **WHEN** a cross struck from near the goal line toward the box is charged down a stride from the crosser
- **THEN** the deflection keeps much of the cross's line, and it can carry over the goal line for a corner

#### Scenario: A read block is directed away from goal
- **WHEN** a defender who had time to read a pass blocks it in front of his own goal
- **THEN** the deflection is sent away from his goal rather than along the pass's line

#### Scenario: A block over his own line is a corner
- **WHEN** a blocked ball crosses the blocker's own goal line wide of the posts
- **THEN** play restarts with a corner to the passing side

### Requirement: The first touch happens at the boot, and it decides the reception
A delivery (pass, cross, clearance or any other ball played to a man) SHALL travel untouched, at its own pace and on its own line, until it is within one stride of the player who takes it — the same reach at which a loose ball is claimed — and under his reach ceiling. The engine SHALL NOT alter a delivery's pace or line because its intended receiver is near it, is running to it, or has been booked as its receiver.

Ownership of a delivery, its first-touch outcome (clean, heavy or miscontrol) and its completion credit SHALL all be decided at that contact, on the same tick, with the pressure and arrival pace read at that moment.

A delivery whose remaining travel will not bring it within that reach of its receiver SHALL stay his to go and get: he moves to it and the ball keeps its own motion. A delivery he never reaches SHALL NOT be credited as received.

A ball coming to its receiver above the deck SHALL be subject to the same rule: it is not controlled until it is within reach across the grass and under his reach ceiling, and then its horizontal pace comes off at the body while it falls to his feet under gravity.

#### Scenario: A pass played through his feet
- **WHEN** a ground pass at 10 m/s is played through a stationary receiver's feet
- **THEN** the ball's pace and heading are unchanged on every tick until it is within one stride of him
- **AND** on the contact tick it becomes his and its pace drops, once, to his own pace
- **AND** it is not touched again before he plays it

#### Scenario: A pass that passes beside him
- **WHEN** a pass's line passes a stride and a half from a stationary receiver
- **THEN** the ball is not turned toward him and runs on untouched
- **AND** the receiver goes to it, and the pass is credited only when he reaches it

#### Scenario: The first touch is rolled where the ball is met
- **WHEN** the receiver meets the ball
- **THEN** the clean, heavy or miscontrol outcome is decided on that tick, with the pressure and arrival pace read then
- **AND** a heavy touch or miscontrol runs on from where the ball was met, never from the receiver's centre

#### Scenario: Taken out of the air
- **WHEN** a lofted ball comes down to its receiver
- **THEN** it is not controlled until it is within one stride across the grass and under his reach ceiling
- **AND** at that contact its horizontal pace comes off and it falls under gravity to his feet

#### Scenario: The recorded track of a reception
- **WHEN** a full match is recorded and its ball track is read at the recorder's 30 ms cadence
- **THEN** a received pass shows no change of pace or line before the contact sample and one pace step at it
- **AND** the ball moves at the receiver's pace from the contact, at no more than a slow walk relative to him

### Requirement: A carried ball moves with its carrier
While a player has the ball under control at his feet, the ball SHALL cross the grass at his pace, and the distance between the ball and his feet SHALL change at no more than a slow walking pace (1.25 m/s) on any axis. A carried ball SHALL NOT be relocated: between any two ticks its position changes only by its own motion.

The same rule SHALL carry a ball in a goalkeeper's hands, with his hands as the anchor and a hand's pace as the bound, so a gathered or caught ball comes into his chest rather than appearing there.

#### Scenario: Moving when he takes it
- **WHEN** a receiver running at 5 m/s takes a pass
- **THEN** on the next tick the ball is moving with him at his pace
- **AND** from then on it stays within the feet radius of him without ever jumping

#### Scenario: He turns
- **WHEN** a carrier turns through 180 degrees
- **THEN** the ball follows him to the point at his feet at no more than the bound
- **AND** it is never written onto his centre

#### Scenario: The keeper's gather
- **WHEN** a keeper gathers a ball off the deck or catches one at full stretch
- **THEN** the ball comes into his chest over a fraction of a second at a hand's pace on every axis
- **AND** it moves with him from then on

### Requirement: A won contact cushions the ball
When a player wins the ball by interception, block or tackle, or is otherwise given a ball at his feet, the ball SHALL leave the contact under his control at his own pace. It SHALL NOT be stopped dead where it was met while he is moving, and it SHALL NOT run on along its old line. The change of pace SHALL happen on the contact tick and only there.

A grant to a player who is not within touch reach of the ball SHALL NOT be a touch: the ball keeps its own motion and he goes to it, or the grant is refused.

Outcomes that leave the ball loose — a deflection, a glance, a knocked-away tackle, a failed first touch — keep their own motion as specified elsewhere.

#### Scenario: An interception on the run
- **WHEN** a defender running across a lane at 5 m/s intercepts a 12 m/s pass
- **THEN** on the next tick the ball is moving with him at about 5 m/s
- **AND** it is neither lying still where it was met nor still travelling along the pass's line

#### Scenario: A standing interceptor
- **WHEN** a standing defender intercepts a pass a stride from his feet
- **THEN** the ball's pace drops to his on the contact tick
- **AND** it closes the last stride to his feet at no more than a slow walking pace, from where it was met

#### Scenario: A controlled block or tackle win
- **WHEN** a block or tackle leaves the winner in control
- **THEN** the ball is carried from the contact exactly as an intercepted ball is

### Requirement: Reception, carry and contact never relocate the ball
On every tick while the ball is in play, no reception, carry, grant or contact SHALL write the ball's position. The ball's position SHALL change only by its own motion — its velocity, the carry expressed as velocity, and collision resolution against a body or the woodwork within the tick's own travel. Placing the ball for a restart and the netting are the only relocations.

#### Scenario: The relocation census
- **WHEN** a batch of matches is run under the whole-tick relocation census
- **THEN** the ball-update and dispatch stages book no relocation at or above the visible threshold outside the restart placement stages

### Requirement: Lofted-ball landing prediction
Players judging where a lofted ball will come down SHALL use a prediction that accounts for air drag and the first bounce, so that a chaser running to the predicted point arrives where the ball actually lands within control range.

#### Scenario: Long ball chased to the right spot
- **WHEN** a long lofted pass is played ahead of a running attacker
- **THEN** the attacker's predicted landing point is within control range of where the ball first becomes playable on the ground

### Requirement: Lane checks consider players
When a player evaluates whether a shooting or passing lane is clear, the lane check SHALL treat opponents on the line as obstructions with a body width consistent with the engine's unit scale, so that a lane through a defender is not reported clear.

#### Scenario: Defender on the shooting line
- **WHEN** a defender stands on the straight line between the ball and the goal within the lane's width
- **THEN** the lane is reported obstructed
