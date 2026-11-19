# core/match/engine/teamplay Specification

## Purpose
Owns team-level tactical organization — the coach who manages each side and the game plan, defensive/attacking shape, and how the team as a whole tracks and marks opposing attackers.

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
When a team is reduced below eleven players by a sending-off or an injury with no change available, the team SHALL re-anchor its shape to a formation for the remaining number of players chosen by the coach's situational logic (sacrificing the role the situation can most afford), SHALL keep that shape across the half-time and extra-time ends swap, and the coach's commitment reading SHALL read the player-count deficit through both sides' chances of scoring.

#### Scenario: Red card reshapes the side
- **WHEN** a side's forward is sent off in the 30th minute while the match is level
- **THEN** the remaining ten re-anchor to a ten-player formation, the vacated role is not left empty, and the same ten-player shape is in force after half time

#### Scenario: Coach reads the deficit
- **WHEN** a side is a player down and leading
- **THEN** its commitment reading is more protective than it would be with eleven men in the same score and minute, from the moment of the sending-off

### Requirement: Offside-trap step-up
A back line holding its line SHALL be able to step up together to leave an attacker offside at the moment a pass is about to be played, with the decision driven by the line's organisation and anticipation, the keeper's organising voice and the pairing chemistry, and with a failure mode in which a late or uncoordinated step-up plays the attacker onside with a clear run.

Every defender who is part of the line SHALL step with it once he has read the call, whichever state he is in. The rule that keeps a defender goal-side of the ball or of his man SHALL NOT pull a stepping defender back.

#### Scenario: Coordinated line springs the trap
- **WHEN** a well-organised back line with high chemistry reads an imminent through ball to an attacker on the shoulder of the last defender
- **THEN** the line steps up together before the ball is played and the attacker is left in an offside position

#### Scenario: Poorly organised line is caught out
- **WHEN** a disorganised back line attempts the same step-up and one defender is late
- **THEN** the attacker is played onside behind the line and the attempt is recorded as a failed trap

#### Scenario: Every man in the line steps
- **WHEN** the line's step is called while one of its defenders is marking, one is covering and one is walking back into his place
- **THEN** each of them steps to the same depth once he has read the call

#### Scenario: The runner's marker steps past him
- **WHEN** the step is called and a defender is marking the runner it is called against
- **THEN** that defender steps past the runner with the line, and is not held goal-side of him

### Requirement: Exactly one side holds possession
At every moment of live play exactly one side SHALL hold possession, and every player on the pitch SHALL read the same answer, whatever his own position relative to the ball. The side holding possession SHALL be:
- the side of the player who has the ball;
- while a restart is waiting to be taken, the side taking it;
- while nobody has the ball, the side that last had it, for as long as its own pass is still travelling to its team-mate;
- once no such pass is travelling, the side that last had it until the other side is decisively closer to the ball.

The direction and speed of the ball SHALL NOT decide possession.

#### Scenario: A backward pass stays in possession
- **WHEN** a side plays a slow or backward pass to a team-mate and nobody has the ball while it travels
- **THEN** that side holds possession throughout the pass, for every one of its players

#### Scenario: Team-mates agree
- **WHEN** the ball is loose and two team-mates stand at different distances from it
- **THEN** both read the same answer about whether their side holds possession

#### Scenario: Only one side claims it
- **WHEN** any live tick of a batch of matches is sampled
- **THEN** exactly one side holds possession, with no tick on which both sides or neither side claim it, against 10,587 and 47,191 such ticks a match before this change

#### Scenario: A loose ball changes hands when it is decisively theirs
- **WHEN** a side's pass is deflected and the ball runs loose with an opponent decisively closer to it than any of that side's players
- **THEN** possession passes to the opponent's side

#### Scenario: A throw-in belongs to the side taking it
- **WHEN** a throw-in has been awarded and is waiting to be taken, wherever the ball and the players are
- **THEN** the side awarded it holds possession

#### Scenario: Possession does not flicker during passes
- **WHEN** 30 equal-strength level-14 matches are played
- **THEN** players' readings of whether their side holds possession change at most 20,000 times a match in total, against 61,560 before this change

### Requirement: The side in possession holds no marking duties
Marking, pressing and covering duties SHALL be assigned only to the side not holding possession. A side SHALL hold no defensive duties while its own pass is travelling, and SHALL receive them on the first tactical refresh after it loses possession.

#### Scenario: A side's own pass in flight
- **WHEN** a side plays a pass to a team-mate and the ball is in the air or rolling
- **THEN** none of its players is assigned a man to mark, and none of them leaves his attacking movement to mark one

#### Scenario: A turnover assigns duties
- **WHEN** a side loses the ball to an opponent
- **THEN** its players are assigned defensive duties within one tactical refresh

#### Scenario: A marker is not sent back and forth by a pass
- **WHEN** 30 equal-strength level-14 matches are played
- **THEN** defenders exchange between marking and general running at most 3,000 round trips a match, against about 17,000 before this change, and a defender's mean spell of marking lasts at least 1.5 s, against 0.6 s before this change

### Requirement: One assignment gives a defender his job
A defender's job against the opposition's possession SHALL come from one assignment made for the whole side: who closes the man on the ball, who covers him, who marks whom, and who holds a zone. A second ordering of the defenders, by their distance to the ball alone, SHALL NOT overrule it.

A defender SHALL also be the man for the carrier, without being assigned, in three situations only: he is one of the two nearest defenders to a carrier in his own penalty area; he is the nearest defender to a carrier within challenge range; or he is the nearest defender to a carrier arriving at the edge of his area.

The assignment SHALL keep a man in a job while he remains nearly as well placed for it as anyone else, for covering as it does for closing. It SHALL NOT send anyone to close a ball held in a goalkeeper's hands. A defender it has given no man SHALL NOT take a man it has given to somebody else.

#### Scenario: A midfielder is closing, and the defender holds
- **WHEN** a midfielder is the side's man on the carrier, 12 m outside the area, and the nearest defender is 8 m from the carrier
- **THEN** the defender keeps his own job and does not leave the line to close the carrier as well

#### Scenario: The cover man keeps the job
- **WHEN** the defender covering the man on the ball is still within 4 m of being the best placed for it
- **THEN** he is still the cover man after the next assignment

#### Scenario: Cover does not change hands faster than closing
- **WHEN** a 30-match batch is played
- **THEN** the covering job changes hands no more often than the closing job does

#### Scenario: A keeper holding the ball is left alone
- **WHEN** the opposition's goalkeeper holds the ball in his hands
- **THEN** no defender is given the job of closing him

#### Scenario: Two defenders on one carrier outside the area
- **WHEN** a 30-match batch is played
- **THEN** the share of time two or more defenders are closing or containing the same carrier outside their own area is lower than before this change

### Requirement: The back line holds one depth
Every defender holding the shape SHALL take his depth from one line, the rear of his side's block, adjusted only by his job: the man covering drops off it, and the man on the far side from the ball tucks back. He SHALL take his width from the line as it squeezes toward its own middle. Where he stood at kick-off SHALL NOT be a depth he steers at. A defender SHALL be treated as wide because of his position in the formation, not because of how far from the middle of the pitch he stands.

#### Scenario: One point, whatever he is doing
- **WHEN** two defenders with the same job and the same place in the formation are in different shape-holding states
- **THEN** both steer at the same point

#### Scenario: A centre-back is not a full-back
- **WHEN** the ball is on a centre-back's side of the pitch
- **THEN** he is not pushed up the way a full-back on that side is

#### Scenario: The line is no looser for it
- **WHEN** a 300-match batch is played
- **THEN** the back line's depth spread is no more than the 12.9 m before this change and its widest gap no more than 15.7 m
- **AND** a centre-back's mean position stays 30 to 40 m from his own goal, and moves 6 to 10 m up the pitch when his side has the ball

### Requirement: The back line drops off an unpressed ball
When an opposing carrier has time on the ball and a pass into the space behind the back line is within his range, the line SHALL drop off. When he is pressed, it SHALL NOT. A carrier has time when no defending player is within 3 m of him. The drop SHALL grow continuously with the time and the range he has, and SHALL be at most 6 m.

#### Scenario: Time on the ball
- **WHEN** a carrier 35 m from goal has nobody within 3 m of him and a runner on the shoulder of the last defender
- **THEN** the back line holds a deeper line than it does for the same carrier with a defender within 3 m of him

#### Scenario: Pressure on the ball
- **WHEN** a defending player gets within 3 m of that carrier
- **THEN** the line stops dropping and comes back to the depth its block gives it

#### Scenario: The drop does not open the game
- **WHEN** a 300-match batch is played
- **THEN** goals, shots and offsides per match stay inside their realism bands

### Requirement: One assignment gives a midfielder his job out of possession
A midfielder's job against the opposition's possession SHALL come from the one assignment made for the whole side: who closes the man on the ball, who covers him, who marks whom, and who holds a zone. The election that decides who chases a loose ball SHALL NOT decide who closes a man who has the ball, except when the assignment has named nobody to close him.

A midfielder SHALL also be the man for the carrier, without being assigned, in two situations only:
- he is one of the two nearest players of his side to a carrier in his own penalty area;
- a carrier comes within challenge range of him, no team-mate is nearer the carrier, and the player assigned to close the carrier is not within challenge range himself.

The second situation SHALL hold for an outfield player of any position, not for defenders alone.

The midfielder given the covering job SHALL stand on the covering point, goal-side of the man closing. A midfielder given no man SHALL NOT take a man the assignment has given to somebody else.

#### Scenario: A carrier runs into a midfielder
- **WHEN** a carrier comes within 2 m of a midfielder, no team-mate is nearer, and the side's assigned closer is 10 m away
- **THEN** the midfielder may contain and challenge him

#### Scenario: The best chaser does not close another man's carrier
- **WHEN** the assignment has named one midfielder to close the carrier and a second midfielder would win a race to a loose ball at the carrier's feet
- **THEN** the second midfielder keeps his own job and does not close the carrier

#### Scenario: Nobody named
- **WHEN** an opponent has the ball and the assignment has named nobody to close him
- **THEN** the best-placed player of the side closes him

#### Scenario: The cover man is on the cover point
- **WHEN** a midfielder is given the covering job
- **THEN** he holds a point goal-side of the man closing the carrier, and keeps it while the job is his

#### Scenario: One man on the carrier in midfield
- **WHEN** a 30-match batch is played
- **THEN** the share of time two or more midfielders are closing or containing the same carrier outside their own area is lower than before this change

### Requirement: The attack's assignment gives a midfielder his job in possession
While a side has the ball, each of its midfielders' jobs SHALL come from the one assignment made for its attack: who runs into the penalty area or in behind, who is the short outlet, who holds each touchline, and who stays behind the ball. A second election of runners, made by the players themselves, SHALL NOT overrule it.

A midfielder the assignment keeps behind the ball SHALL stay behind it. A midfielder it names for nothing SHALL hold his place in the shape. The short outlet SHALL be named whenever the side has the ball, in every phase of its possession.

#### Scenario: Kept behind the ball
- **WHEN** the assignment keeps four players behind the ball and one of them is a central midfielder
- **THEN** that midfielder does not take up a place in the penalty area, whatever his own attacking attributes

#### Scenario: No run without a job
- **WHEN** a midfielder has been given no run in the current attack
- **THEN** he does not arrive in the penalty area in open play

#### Scenario: An outlet in the build-up
- **WHEN** a side has the ball in its own third and is not yet committing players forward
- **THEN** one of its players is named as the short outlet, and he shows for the ball

#### Scenario: Two central midfielders are two players
- **WHEN** a 300-match batch is played in a formation with two central midfielders
- **THEN** the two of them are inside the opposition's penalty area at the same moment less often than before this change, and their two position maps are less alike than the 0.74 before it

#### Scenario: Holding men back does not close the game
- **WHEN** the same batch is played
- **THEN** goals, shots and expected goals per match stay inside their realism bands

### Requirement: A midfielder holding the shape stands on one place
Every midfielder state that holds the shape SHALL steer at the place his side's shape gives him at that moment. Where he stood at kick-off SHALL NOT be a place he steers at. It MAY still decide which place in the formation is his.

#### Scenario: One point, whatever he is doing
- **WHEN** two midfielders with the same job and the same place in the formation are in different shape-holding states
- **THEN** both steer at the same point

#### Scenario: The block has moved
- **WHEN** his side's shape has moved 20 m up the pitch from where it stood at kick-off
- **THEN** a midfielder holding the shape stands where the shape now puts him, and does not run back toward where he stood at kick-off

#### Scenario: The midfield is no looser for it
- **WHEN** a 300-match batch is played
- **THEN** a central midfielder's mean position moves 8 to 14 m up the pitch when his side has the ball, and his position along the pitch varies by no more than the 23 m before this change

### Requirement: One assignment gives a forward his job out of possession
A forward's job against the opposition's possession SHALL come from the one assignment made for the whole side. That assignment MAY name a forward to close the man on the ball. It SHALL NOT give a forward a man to mark, the covering job or a zone to hold. The election that decides who chases a loose ball SHALL NOT decide which forward closes a man who has the ball, except when the assignment has named nobody to close him.

A forward SHALL also be the man for the carrier, without being assigned, in the situations in which any outfield player is: he is one of the two nearest players of his side to a carrier in his own penalty area, or a carrier comes within challenge range of him with no team-mate nearer and the assigned closer not within challenge range himself.

A forward who is not the man for the carrier SHALL hold his place at the front of his side's shape.

#### Scenario: The best chaser does not close another man's carrier
- **WHEN** the assignment has named a midfielder to close the carrier and a forward would win a race to a loose ball at the carrier's feet
- **THEN** the forward holds his place and does not close the carrier

#### Scenario: A forward the assignment names closes the carrier
- **WHEN** the assignment names a forward to close a centre-back on the ball
- **THEN** that forward closes him, and the other forward holds his place

#### Scenario: Named in his own half
- **WHEN** the assignment names a forward to close a carrier who is in the forward's own half
- **THEN** he closes him, as he would in the opposition's half

#### Scenario: Nobody named
- **WHEN** an opponent has the ball and the assignment has named nobody to close him
- **THEN** the best-placed player of the side closes him, and that player may be a forward

#### Scenario: One man from the front
- **WHEN** a 30-match batch is played
- **THEN** the share of time two forwards are closing or containing the same carrier is lower than before this change

### Requirement: The attack's assignment gives a forward his job in possession
While a side has the ball, each of its forwards' jobs SHALL come from the one assignment made for its attack: who makes the run in behind, who attacks each place in the penalty area, who is the short outlet, and who holds each touchline. A second election, made by the players themselves, SHALL NOT overrule it. A forward the assignment names for nothing SHALL hold his place on his side's front line.

The assignment SHALL name at most one runner in behind. It SHALL name him whenever the side has the ball and there is space behind the opposition's last line to run into, in every phase of its possession, and before it fills the places in the penalty area. It SHALL NOT name one when the opposition's last line stands on its own penalty area. The player the passer is told to look for in behind and the player who makes the run SHALL be the same player.

A place in the penalty area SHALL take a player out of the other jobs from the moment the attack on the penalty area begins. Until then the player who will attack it MAY be named the short outlet.

#### Scenario: The passer and the runner agree
- **WHEN** the assignment names a runner in behind
- **THEN** the player making the run and the player the passer is told to look for are the same man

#### Scenario: One runner
- **WHEN** a side with two strikers has the ball in midfield with 30 m of space behind the opposition's last line
- **THEN** exactly one player is named to run in behind

#### Scenario: A striker can be the runner
- **WHEN** the quickest attacking player within reach of the goal is a striker who would also suit a place in the penalty area
- **THEN** he is named the runner, and the place goes to somebody else

#### Scenario: No space, no runner
- **WHEN** the opposition's last line stands on the edge of its own penalty area
- **THEN** nobody is named to run in behind, and the places in the penalty area are filled as before

#### Scenario: A runner in the build-up
- **WHEN** a side has the ball in its own third, is not yet committing players forward, and the opposition's last line is near halfway
- **THEN** one of its players is named the runner in behind, and he waits on the shoulder of the last defender

#### Scenario: A striker shows before he attacks the box
- **WHEN** a striker who will attack a place in the penalty area is the nearest free player to the ball, and the ball is still in midfield
- **THEN** he may be named the short outlet
- **AND** once the attack on the penalty area begins he attacks his place, and somebody else is the outlet

#### Scenario: Naming a runner does not empty the box
- **WHEN** a 300-match batch is played
- **THEN** the mean number of the attacking side's players inside the penalty area when a shot is struck is no lower than before this change
- **AND** goals, shots and expected goals per match stay inside their realism bands

### Requirement: A forward holding the shape stands on one place
Every forward state that holds the shape SHALL steer at the place his side's shape gives him at that moment. Where he stood at kick-off SHALL NOT be a place he steers at, and where the ball is SHALL NOT move that place by a rule of his own. His kick-off position MAY still decide which place in the formation is his.

Which of two forwards takes which side of the pitch SHALL follow their places in the formation. It SHALL NOT follow the order of their identifiers.

#### Scenario: One point, whatever he is doing
- **WHEN** two forwards with the same job and the same place in the formation are in different shape-holding states
- **THEN** both steer at the same point

#### Scenario: The block has moved
- **WHEN** his side's shape has moved 20 m up the pitch from where it stood at kick-off
- **THEN** a forward holding the shape stands where the shape now puts him, and does not drift back toward where he stood at kick-off

#### Scenario: The left striker is on the left
- **WHEN** two strikers hold their places with the ball in the centre of the pitch
- **THEN** the one whose place in the formation is on the left stands to the left of the other

#### Scenario: The front line is no looser for it
- **WHEN** a 300-match batch is played
- **THEN** a striker's position along the pitch varies by no more than the 18.4 m before this change
- **AND** his mean position is reported against the real 65 to 78 m from his own goal

### Requirement: One coach manages each side
Each side SHALL be managed for the whole match by one coach. Every team-level decision the side makes SHALL be that coach's: the plan it starts with, how much it commits as the match goes on, when and how the plan changes, the half-time talk, and what the bench is asked for. No other part of the match SHALL make those decisions for the side.

The coach's temperament SHALL come from the side's head coach:
- how much risk the coach accepts, and how cautious the coach is;
- how stubborn or adaptable the coach is;
- how strongly the coach reacts to what has just happened;
- how well the coach reads a match;
- how well the coach handles and motivates players.

A side with no head coach SHALL be managed by a coach drawn from the population of coaches by the match seed and the side. Two such sides SHALL NOT share one temperament, and no side SHALL be managed by an average coach.

#### Scenario: Two coaches, one side
- **WHEN** the same fixture is played with the same squads and seed, once under a cautious head coach and once under a risk-taking one
- **THEN** the side's plan and its commitment differ between the two matches
- **AND** the risk-taking coach's side commits more when it is behind

#### Scenario: A side with no head coach
- **WHEN** harness matches are played between sides with no head coach
- **THEN** the coaches drawn for different seeds differ in temperament, and the same seed draws the same coaches

#### Scenario: The bench and the pitch answer to one coach
- **WHEN** a side is chasing the match
- **THEN** the change its bench is asked for and the way its players are asked to play come from the same reading of the match

### Requirement: The plan is the coach's, not the formation's
At kickoff the coach SHALL set the side's plan. The plan SHALL be a set of continuous settings that do not depend on the formation:
- how high the side defends;
- how hard and how high it presses, including straight after losing the ball;
- how compact it stays out of possession;
- how wide it plays in possession;
- how quickly it moves the ball;
- how directly it plays forward;
- how many players it commits forward and how many it keeps back.

The plan SHALL come from four things:
- the coach's preferred way of playing;
- the two elevens' relative strength;
- the venue;
- the occasion.

It SHALL then be fitted to what the eleven can carry out.

The formation SHALL decide only where each player stands within the plan. Width in possession SHALL NOT be derived from compactness out of possession.

#### Scenario: One formation, two plans
- **WHEN** two coaches with different preferred football field the same eleven in the same formation against the same opponent
- **THEN** their sides defend at different heights and press with different intensity

#### Scenario: The underdog away
- **WHEN** the same coach and eleven play once as a clear favourite at home and once as a clear underdog away
- **THEN** as the underdog the side defends deeper, stays more compact and commits fewer players forward

#### Scenario: A plan the eleven cannot play
- **WHEN** a coach who prefers to keep the ball has an eleven that cannot pass well under pressure
- **THEN** the side plays more directly than that coach's preference

#### Scenario: Compact without and wide with the ball
- **WHEN** a side's plan is compact out of possession and wide in possession
- **THEN** its width in possession is the width its plan asks for, whatever its compactness out of possession

#### Scenario: The plan does not widen the formation spread
- **WHEN** 400 matches are played between mirrored sides in each of 4-4-2, 4-3-3, 4-2-3-1 and 3-5-2, once on the coaches' plans and once with both coaches pinned to the same neutral plan
- **THEN** each shape's goals per match on the coaches' plans differ from its neutral figure by less than two standard errors of the difference, and the gap between the highest- and lowest-scoring shapes is no wider on the coaches' plans than on the neutral plan

### Requirement: The stance follows the match from the moment the score changes
The coach SHALL keep one continuous reading of how much the side should commit. That reading SHALL weigh what a goal for and a goal against are worth to the side between now and the final whistle, from:
- the score and the time left;
- both sides' chances of scoring, from their relative strength, the venue and the players each has on the pitch;
- the occasion: league points, a knockout tie decided on the day, or a friendly.

The coach's temperament SHALL bend the reading. The reading SHALL see the real score from the moment a goal is scored. It SHALL NOT branch on the size of a lead, and it SHALL NOT wait for a minute of the match.

#### Scenario: Behind commits more than level
- **WHEN** the same side is level at one minute of a match and one goal behind at the same minute of another
- **THEN** it commits more when it is behind

#### Scenario: A late lead is protected harder than an early one
- **WHEN** a side leads by one goal in the 20th minute and, in another match, in the 80th minute
- **THEN** it commits less in the 80th minute than in the 20th
- **AND** it commits less leading in the 20th minute than it does level in the 20th minute

#### Scenario: The score is read at once
- **WHEN** a side goes a goal behind in the 15th minute
- **THEN** its commitment changes within the next reading of the match, not after the hour

#### Scenario: The favourite pushes for the win and the underdog takes the point
- **WHEN** a clear favourite and a clear underdog are level in the 80th minute of a league match
- **THEN** the favourite commits more than the underdog

#### Scenario: A tie decided on the day
- **WHEN** two equal sides are level in the 85th minute, once in a league match and once in a knockout tie that goes to extra time on a draw
- **THEN** they commit less in the knockout tie than in the league match

#### Scenario: Temperament bends the reading
- **WHEN** a risk-taking and a cautious coach are each one goal behind in the 70th minute with equal sides
- **THEN** the risk-taking coach's side commits more

#### Scenario: Equal sides keep the score honest
- **WHEN** 600 equal-strength level-14 matches are played
- **THEN** the two sides' goal counts correlate between −0.08 and +0.08
- **AND** draws are within 3 percentage points of what independent scoring at the batch's own rates would give, and are 23% to 30% of matches
- **AND** a leading side's scoring rate is between 0.75 and 1.33 times a trailing side's, both before and after the 62nd minute

#### Scenario: A lead does not keep the ball to itself
- **WHEN** the same batch is played
- **THEN** a side leading by one goal holds no more than 2 percentage points more possession than it does when level, and no more than 10 points less

### Requirement: The stance moves only levers a side really uses
The coach's reading of how much to commit SHALL change how the side plays only through:
- the height of its block and defensive line;
- the number of players it commits forward and keeps back;
- how high and how hard it presses;
- the risk it takes with its passing;
- the time it takes over its restarts.

It SHALL NOT change how fast any player runs, whether a player shoots at a chance, or how long a player must keep the ball before passing.

#### Scenario: Protecting a lead at full pace
- **WHEN** a side protecting a late lead loses the ball and its defenders run back toward their goal
- **THEN** they run at the pace they would with the score level

#### Scenario: A leading side still shoots
- **WHEN** a player of a side protecting a lead has a clear chance to shoot
- **THEN** the decision to shoot is weighed exactly as it would be with the score level

#### Scenario: Deeper to protect
- **WHEN** a side protects a one-goal lead in the 85th minute
- **THEN** its defensive line stands deeper, and it keeps more players behind the ball, than at the same minute with the score level

#### Scenario: Restarts slow down
- **WHEN** a side protects a lead late in a match
- **THEN** its restarts take longer than at the same minute with the score level, and longer the closer the final whistle is

#### Scenario: No wait before a pass
- **WHEN** a player of a side protecting a lead receives the ball with a team-mate open
- **THEN** the player may pass it at once

### Requirement: The coach changes the plan at the moments coaches act
The coach SHALL reconsider the plan, both the formation and its settings, at these moments only:
- half-time;
- after a goal for or against;
- after a sending-off;
- after a forced substitution;
- when the coach's read of the match says the plan is failing.

The commitment reading SHALL keep moving the settings between those moments, and SHALL move them by the same amount whatever plan is in force. The formation and the settings the coach chose SHALL change only at a moment.

A change of settings SHALL answer how the side is playing against what the coach expected of it: it asks less of a side that is being outplayed and more of one that is on top. How readily the coach changes the plan SHALL follow the coach's stubbornness and adaptability. How far the change goes SHALL follow how well the coach reads a match.

One side's change SHALL NOT delay or block the other side's.

When the formation changes, every player on the pitch SHALL take a place in the new formation. The side SHALL decide who goes where as it does for a substitution.

#### Scenario: A change at half-time
- **WHEN** a side trails a weaker side at half-time
- **THEN** its coach may change the plan at the interval, and the change is in force from the second-half kickoff

#### Scenario: No change between moments
- **WHEN** no goal, sending-off, forced substitution or failing read occurs between the 20th and the 40th minute
- **THEN** neither side's formation nor its chosen settings change in that span

#### Scenario: A change keeps the push
- **WHEN** the same side is one goal behind in the 70th minute of two matches, having changed its plan at half-time in one and not in the other
- **THEN** the score moves its line, its press, the men it commits and its passing risk by the same amount in both

#### Scenario: An outplayed side is asked for less
- **WHEN** a side has made fewer chances and conceded more than its coach expected over his read, and he changes the plan
- **THEN** the new plan holds a deeper block, presses less and commits fewer men than the plan it replaces

#### Scenario: A sharp reader answers further
- **WHEN** a sharp reader of a match and a poor one see the same read of the same side at half-time
- **THEN** the sharp reader's new plan moves further from the old one

#### Scenario: Stubborn and adaptable coaches
- **WHEN** a batch of matches is played in which a stubborn coach and an adaptable coach each face a plan that is failing
- **THEN** the adaptable coach changes the plan sooner, on average

#### Scenario: A change of shape moves the players
- **WHEN** a side changes from a back four to a back five
- **THEN** a player takes up each place in the back five, and no player stays on a place the new formation does not have

#### Scenario: Each side changes on its own clock
- **WHEN** one side changes its plan in the 60th minute and the other side's coach reaches a moment in the 62nd
- **THEN** the second side's change is made in the 62nd minute

#### Scenario: Changes happen through the match
- **WHEN** 600 matches are played
- **THEN** half-time is the most common single moment for a plan change, and changes are spread through the match rather than gathered after the 85th minute

### Requirement: The coach reads a moving window of the match
The coach's read of the run of play SHALL cover the same trailing span of play at every moment once that span has been played, and SHALL never restart from nothing. The read covers chances created and conceded, territory, possession, pressing success, dangerous turnovers and line breaks.

A line break SHALL be counted whenever the opposition, in open play, passes or carries the ball past the side's last line.

#### Scenario: The read does not reset
- **WHEN** the coach reads the match in the 16th minute and again in the 31st minute
- **THEN** each read covers the 15 minutes of play before it

#### Scenario: Possession read in range
- **WHEN** a side has held the ball for half of the window
- **THEN** its read of possession is one half, whichever moment of the match the window ends at

#### Scenario: A line break is counted
- **WHEN** an opponent completes a pass in open play to a team-mate running beyond the side's last line
- **THEN** the side's read counts one line break

### Requirement: Each player follows the plan as far as that player can and will
How closely a player follows the plan SHALL follow one reading of that player. Following the plan means standing in the plan's place, joining the press and staying in it, and taking up the run or the cover the plan gives. The reading SHALL come from:
- the player's understanding of the game: decisions, positioning, anticipation and concentration;
- teamwork and work rate;
- condition;
- familiarity with the role being played;
- the side's familiarity with the plan;
- buy-in to the coach.

The reading SHALL act through three things only: the place the player holds, the press the player joins, and the jobs the player takes up. It SHALL NOT change any of the player's skills.

#### Scenario: The press fades with the legs
- **WHEN** the same player is in a pressing plan, once fresh and once tired
- **THEN** the player joins fewer of the side's presses when tired

#### Scenario: Work rate keeps a player in the press
- **WHEN** two players of the same side differ only in work rate, in a pressing plan
- **THEN** the harder-working player joins more of the side's presses

#### Scenario: Out of position
- **WHEN** a centre-back plays as a central midfielder
- **THEN** that player holds the plan's place less closely than a natural central midfielder with the same attributes would

#### Scenario: Skills are untouched
- **WHEN** a player follows the plan loosely
- **THEN** the player's passes, tackles and shots are decided by the same skills as when following it closely

#### Scenario: A squad is not uniform
- **WHEN** a batch of level-14 matches is played
- **THEN** players' readings spread across each squad
- **AND** goals, shots and the shape bands stay inside their realism bands

### Requirement: A side's familiarity with its shape is earned
A side's familiarity with its formation SHALL come from its own record:
- how many of its recent matches it started in that formation;
- how much the eleven on the pitch have played together in it.

A side that changes formation during a match SHALL play the new formation at the familiarity its record gives that formation.

A player SHALL count as new to the team-mates the player has not yet played with. A substitute SHALL be read by the same rule as a starter.

#### Scenario: A settled side
- **WHEN** one side has started its last ten matches in a 4-4-2 with mostly the same eleven, and another side switched to a 4-4-2 this week
- **THEN** the settled side's familiarity is the higher

#### Scenario: A change of shape during the match
- **WHEN** a side that has rarely played a 3-5-2 switches to one during a match
- **THEN** it plays the 3-5-2 at a lower familiarity than the shape it left
- **AND** its back line steps up together less reliably than before the switch

#### Scenario: A new signing
- **WHEN** a player plays a first match for a club
- **THEN** that player's partnerships with the team-mates count as new

#### Scenario: A substitute joins the partnerships
- **WHEN** a substitute comes on
- **THEN** the substitute's partnerships with the players on the pitch come from the substitute's record with them

### Requirement: The coach gives the half-time talk at the interval
At half-time the coach SHALL give the side's team talk. The talk SHALL be given in the match, before the second half starts. Its tone SHALL follow two things: the score against what the coach expected of the match, and the coach's temperament. The talk the players react to SHALL be the talk recorded with the result.

#### Scenario: The tone follows the half
- **WHEN** a side trails a weaker side at half-time, and in another match leads the same side by two goals
- **THEN** the coach's tone differs between the two half-times

#### Scenario: The talk is recorded
- **WHEN** a match reaches half-time
- **THEN** the result records each side's half-time talk
