# core/match/engine/officiating Specification

## Purpose
Owns the referee: restart set-up, foul and disciplinary decisions, advantage play, and the composed home-advantage effect applied through officiating bias.

## Requirements

### Requirement: Kickoff and restart set-up
At the start of each period and after every goal, the system SHALL position all players according to a restart shape before play resumes: the team taking the restart has a designated taker and, when a team-mate is available, a nearby partner to receive the first pass, while the opposing team is kept a minimum distance from the ball.
Other dead-ball restarts (throw-ins, corners, goal kicks, free kicks, penalties, drop balls) SHALL likewise position the relevant players before the ball is put back into play. For a free kick within striking range the defending side SHALL form a wall of a size appropriate to the distance, standing at the restraining distance on the line between ball and goal, and every other opponent SHALL retreat to the restraining distance. For a penalty every player other than the taker and the keeper SHALL stand outside the penalty area and arc until the kick is struck. For a throw-in at least one team-mate SHALL offer a short option within throwing range.

#### Scenario: Kick-off has a receiving option
- **WHEN** a team with at least two outfield players available takes a kickoff
- **THEN** the taker is not left isolated — a team-mate is stationed nearby as a pass option and the opposing team is kept outside the required restraining distance

#### Scenario: Kick-off with only a goalkeeper outfield
- **WHEN** the kicking side has no available outfield team-mate besides the taker
- **THEN** no partner station is assigned and the taker proceeds without one

#### Scenario: Wall forms for a direct free kick in range
- **WHEN** a direct free kick is awarded 22 m from goal
- **THEN** a wall of defenders stands at the restraining distance between the ball and the goal before the kick is taken, no other opponent is inside that distance, and the wall-block chance is read from the wall that actually formed

#### Scenario: Penalty area is clear at a penalty
- **WHEN** a penalty is awarded
- **THEN** no player other than the taker and the keeper is inside the penalty area or the arc when the kick is struck

### Requirement: Foul-call probability on tackle contact
The referee SHALL assess contact during a tackle for a foul, using the severity of the contact, and SHALL escalate the resulting restart to a penalty when the foul occurs inside the defending team's penalty area.

#### Scenario: Reckless contact raises foul-call probability
- **WHEN** a tackle attempt is classified as reckless-severity contact rather than normal contact
- **THEN** the probability the referee calls a foul on that contact is higher than for normal-severity contact

#### Scenario: Contact inside the penalty box can be a penalty
- **WHEN** foul contact against an attacking player occurs inside the defending team's penalty area and the referee's whistle goes
- **THEN** the resulting restart is a penalty kick rather than a free kick

### Requirement: Officiating — foul detection and disciplinary action
The referee SHALL assess contact during duels for a foul. The decision to stop play SHALL use:
- the severity of the contact;
- the referee's own strictness/leniency profile;
- situational modifiers: location on the pitch, match temperature, and a bounded home-team bias scaled by crowd intensity.

The referee SHALL be able to issue yellow or red cards for qualifying conduct. The restart for a foul SHALL be awarded where the contact happened, not where the ball is when the whistle goes.

Beyond tackle contact, the referee SHALL recognise the following offences, each with its own card logic and census source:
- a professional foul: yellow when it stops a promising attack, red when it denies an obvious goal-scoring opportunity;
- a goalkeeper's foul in a one-on-one that denies an obvious goal-scoring opportunity:
  - inside his own penalty area, when he genuinely attempted to play the ball, a penalty kick and a yellow card;
  - outside his penalty area, a free kick and a red card;
- time-wasting by a leading side once a cumulative delay threshold is passed (yellow);
- dissent after a decision by a player of low temperament (yellow);
- a handball when a block or clearance is made with the arm: a free kick or penalty, with a card when it denies a goal or an obvious chance.

A player already booked SHALL be less willing to engage in a risky challenge, not merely less likely to foul once committed, in every position group.

#### Scenario: Match temperature raises foul-call sensitivity
- **WHEN** the match has recently accumulated fouls, cards, or heated incidents (elevated match temperature)
- **THEN** subsequent borderline contact is more likely to be called a foul than the same contact earlier in a calm match

#### Scenario: Two yellow cards produce a sending-off
- **WHEN** a player receives a second yellow card in the same match
- **THEN** that player is sent off and recorded as a red card

#### Scenario: Professional foul stops a counter
- **WHEN** a carrier breaks with an open lane and fewer than two defenders between the ball and the goal, and the nearest defender is within fouling range
- **THEN** the defender may deliberately foul with a probability driven by his aggression, decisions and sportsmanship, the side's desperation and the match minute, and the resulting card is red when the foul denies an obvious goal-scoring opportunity close to goal and yellow otherwise

#### Scenario: A keeper brings down a man through on goal in his area
- **WHEN** a goalkeeper going to ground at the feet of a striker who is through on goal misses the ball and brings him down inside the penalty area
- **THEN** a penalty kick is awarded and the goalkeeper is cautioned, not sent off

#### Scenario: A keeper brings down a man through on goal outside his area
- **WHEN** a goalkeeper who has come out of his area brings down a striker who is through on an open goal
- **THEN** a direct free kick is awarded and the goalkeeper is sent off

#### Scenario: Booked defender declines a risky challenge
- **WHEN** a defender on a yellow card faces the same engagement decision as an unbooked team-mate of identical attributes
- **THEN** the booked defender's probability of committing to the challenge is lower

#### Scenario: Dissent after a decision
- **WHEN** a decision goes against a player with low temperament and high aggression
- **THEN** that player may be booked for dissent with a small probability that rises with the referee's card-happiness

### Requirement: Advantage and set-piece restarts
The referee SHALL be able to play advantage rather than immediately stopping play for a foul. When advantage is pulled back, the restart SHALL be at the original foul spot, and the original card decision SHALL be preserved even if a second foul occurs during the advantage window.

Every stoppage (foul, offside, ball out of play, injury) SHALL resume via the appropriate restart type (direct free kick, indirect free kick, throw-in, corner, goal kick, penalty, drop ball), with players positioned accordingly. A restart SHALL NOT be given to a player who is off the pitch or lying injured.

An indirect free kick SHALL be awarded for offside, and a goal SHALL NOT be scored directly from it.

A goalkeeper who controls the ball in his hands for more than eight seconds SHALL concede a corner kick (Law 12 since 2025). The eight seconds SHALL be counted from the moment he takes control of the ball in his hands, whatever he is doing in that time.

#### Scenario: Advantage played after a foul
- **WHEN** the fouled team retains a promising attacking position immediately after the contact
- **THEN** the referee can delay the whistle to let play continue before resolving the foul call

#### Scenario: Advantage pulled back
- **WHEN** advantage is played and the fouled side loses the ball inside the advantage window
- **THEN** play restarts with a free kick at the spot of the original foul and any card for that foul is issued

#### Scenario: Keeper holds the ball too long
- **WHEN** a goalkeeper slowing the game keeps the ball in his hands for more than eight seconds
- **THEN** the referee awards a corner kick to the opponents

#### Scenario: The count starts when he takes the ball
- **WHEN** a keeper catches the ball in a dive, stays down with it and then carries it out
- **THEN** his eight seconds are counted from the catch, not from when he got up

#### Scenario: Indirect free kick cannot score directly
- **WHEN** an indirect free kick is struck straight into the goal without touching another player
- **THEN** no goal is awarded and play restarts with a goal kick

#### Scenario: Injury stoppage restarts with a drop ball
- **WHEN** the referee stops play for an injured player while the ball is in play
- **THEN** play restarts with a drop ball to the team that last had possession, at the place the ball was when play stopped, with a keeper drop when that place is inside the penalty area

#### Scenario: The injured keeper is not given the drop ball
- **WHEN** play is stopped for an injury to the goalkeeper while the ball is in his penalty area
- **THEN** the drop ball goes to a team-mate of his, not to the injured keeper

#### Scenario: A sent-off keeper takes no goal kick
- **WHEN** a goal kick is awarded to a side whose goalkeeper has been sent off
- **THEN** the goal kick is taken by the player now in goal, never by the sent-off keeper

### Requirement: Home advantage as a composed effect
Home advantage SHALL be expressed through multiple distinct channels (e.g. crowd-driven referee bias, environmental/psychological modifiers) that can each be independently isolated or disabled for testing, rather than as a single undifferentiated home bonus.

#### Scenario: Disabling one home-advantage channel does not remove the others
- **WHEN** the referee's crowd-driven bias channel is held flat for test purposes
- **THEN** other home-advantage channels in the match continue to operate independently

### Requirement: Time-wasting by a leading side
A side protecting a lead late in the match SHALL be able to drag out its own restarts (throw-ins, goal kicks, free kicks, substitutions) by an amount scaled by how far its coach's commitment reading has turned toward protecting the lead, which grows as the final whistle nears, and by the players' temperament; that delay SHALL count as dead-ball time for stoppage purposes, and the referee SHALL book a player for time-wasting once the side's cumulative delay in the period exceeds the referee's tolerance, with a rising probability on repeat offences.

#### Scenario: Leading side slows its restarts
- **WHEN** a side is leading in the 80th minute and its coach's reading has turned toward protecting the lead
- **THEN** its throw-ins and goal kicks are taken later than the same restarts taken by a level side, and the extra dead time is added to that period's stoppage allowance

#### Scenario: Persistent time-wasting is booked
- **WHEN** a leading side's cumulative restart delay in a period exceeds the referee's tolerance
- **THEN** the next delayed restart carries a probability of a yellow card for the taker that grows with the referee's strictness and the number of prior offences

### Requirement: A goal kick is taken once its set-up is formed
Once the taker has reached the ball, a goal kick SHALL wait for a short placing beat and then SHALL be taken as soon as its set-up is formed. The set-up is formed when no opponent is inside the taker's penalty area (Law 16) and the taker's side has stopped moving into position. The wait SHALL NOT be a fixed duration that the state of the pitch cannot shorten. A ceiling SHALL bound the wait so that a set-up that never forms cannot stall the match; the ceiling is no shorter than the previous fixed wait (20 s from the taker reaching the ball). A goal kick taken off a run-up SHALL start its run on the same test, after the taker's look up from his mark. A leading side's time-wasting delay SHALL still apply on top, as the time-wasting requirement describes.

#### Scenario: Kicked once the picture is set
- **WHEN** the keeper has reached a goal kick, no opponent is inside his penalty area, and his team-mates have been still for a second
- **THEN** the goal kick is taken within the placing beat of that moment, well before 20 s have passed since he reached the ball

#### Scenario: An opponent in the area holds the kick
- **WHEN** an opponent is still inside the taker's penalty area after the placing beat has passed
- **THEN** the goal kick is not taken until he has left the area or the ceiling expires

#### Scenario: Still arranging holds the kick
- **WHEN** the taker's team-mates are still moving into position after the placing beat has passed
- **THEN** the goal kick is not taken until they have stopped moving or the ceiling expires

#### Scenario: A set-up already formed still has its placing beat
- **WHEN** the set-up is already formed on the tick the keeper reaches the ball
- **THEN** the goal kick is not taken on that tick, but once the placing beat has passed

#### Scenario: A set-up that never forms is bounded
- **WHEN** the set-up does not form after the keeper reaches the ball
- **THEN** the goal kick is taken no later than the ceiling

#### Scenario: The run-up starts when the picture is set
- **WHEN** the keeper has chosen to go long and is standing on his run-up mark, and the set-up becomes formed after his look up
- **THEN** he starts his run at that point rather than at the end of a fixed wait

#### Scenario: No frozen picture at goal kicks
- **WHEN** a batch of level-14 matches is played
- **THEN** the median longest stretch during a goal kick in which every other player stands still is at most 2 s, compared with 5.3–8.7 s before this change

### Requirement: The free-kick wall covers the near post and the keeper covers the rest
At a direct free kick within shooting range, the defending wall SHALL be lined up to cover the part of the goal nearer the ball. Its outermost man SHALL stand on the line from the ball to the near post, and the rest of the wall SHALL extend inward from him. The goalkeeper SHALL position himself to cover the part of the goal the wall leaves open, reading the wall where it actually stands.

#### Scenario: A free kick from a wide angle
- **WHEN** a direct free kick is awarded 20 m from goal, level with one of the posts
- **THEN** the wall covers the near-post side of the goal, and the keeper stands toward the far post with the ball in his view

#### Scenario: The keeper reads the wall
- **WHEN** the kick is taken before the wall's men have reached their stations
- **THEN** the keeper covers the part of the goal the men actually standing in the wall leave open

### Requirement: A foul is placed on the player it was committed on
The player fouled SHALL be the one the contact was made on: the carrier for a challenge on the ball, and the player being held for a holding foul. The restart, a penalty and any injury from the contact SHALL follow that player and his position, not the ball carrier's when the two differ.

#### Scenario: A shirt-pull in the area while the ball is outside it
- **WHEN** a defender holds an attacker inside the defending penalty area while another attacker carries the ball outside it, and the referee whistles
- **THEN** the restart is a penalty kick

#### Scenario: A shirt-pull outside the area while the ball is inside it
- **WHEN** a defender holds an attacker 30 m from goal while another attacker carries the ball inside the penalty area, and the referee whistles
- **THEN** the restart is a free kick where the attacker was held, not a penalty kick

#### Scenario: The man held is the man hurt
- **WHEN** a holding foul carries an injury impulse
- **THEN** the impulse goes to the player who was held

#### Scenario: Penalties stay in their band
- **WHEN** a 300-match batch is played
- **THEN** penalties awarded per match are within their realism band

### Requirement: An outfield player's foul that denies an obvious goal-scoring opportunity is judged as one
A tackle or a holding foul by an outfield player SHALL be assessed for denying an obvious goal-scoring opportunity, whether or not it was a deliberate professional foul. When it is judged to have denied one:
- outside his own penalty area, the restart SHALL be a free kick and he SHALL be sent off;
- inside it, when he genuinely attempted to play the ball, the restart SHALL be a penalty kick and he SHALL be cautioned;
- inside it, when he did not attempt to play the ball, the restart SHALL be a penalty kick and he SHALL be sent off.

Whether an opportunity is obvious SHALL be assessed on the carrier's distance from goal, the lane to it and the outfield defenders who can still cover. The goalkeeper SHALL NOT count as a covering defender. A deliberate professional foul SHALL be held back inside a player's own penalty area as his other fouls are.

#### Scenario: The last man's tackle outside the area
- **WHEN** the last outfield defender fouls a carrier who is through on goal with a clear lane, 25 m out
- **THEN** the foul is assessed for denying an obvious goal-scoring opportunity, and when it is judged to have denied one he is sent off

#### Scenario: A genuine attempt in the area
- **WHEN** the same foul is a tackle made inside his own penalty area
- **THEN** the restart is a penalty kick and he is cautioned, not sent off

#### Scenario: The keeper is not cover
- **WHEN** only the goalkeeper stands between the fouled carrier and the goal
- **THEN** the opportunity is assessed as having no covering defender

#### Scenario: Not in his own area
- **WHEN** a defender could stop a break with a professional foul inside his own penalty area
- **THEN** he is far less likely to commit it than he would be outside the area

#### Scenario: Red cards stay in their band
- **WHEN** a 300-match batch is played
- **THEN** red cards and penalties per match are within their realism bands
