# core/match/engine/player Specification

## Purpose
Owns individual player behavior on the pitch: passing and shooting decisions, tackling, goalkeeper actions, fatigue/condition, injury state, and the per-player stat line those actions accumulate into.

## Requirements

### Requirement: Ball possession and passing decisions
A player in possession SHALL evaluate candidate passes using a combination of distance, passing angle, pressure on the passer, the receiver's positioning and ability, the passer's own ability, and the tactical value of the resulting position, and SHALL weigh the risk that the pass is intercepted or blocked before choosing whether and where to pass. The risk of a block SHALL be the same chance the block contest rolls for that pass from the same positions.

#### Scenario: Pass success probability responds to pressure
- **WHEN** the passer is under close defensive pressure at the moment of the pass
- **THEN** the evaluated success probability for that pass is lower than the same pass attempted with no pressure applied

#### Scenario: Adverse weather reduces pass accuracy
- **WHEN** the match environment carries adverse weather (rain or wind) modifiers
- **THEN** evaluated pass success probability is reduced accordingly, with long passes affected further by wind

#### Scenario: A pass through a man at point blank is priced
- **WHEN** a candidate pass's line runs through an opponent a stride from the passer, below that opponent's standing reach
- **THEN** the evaluated risk of that pass includes the chance of it being charged down, and is higher than for the same pass with him off the line

### Requirement: Shot decision and shot type classification
An attacking player in possession within shooting range SHALL decide whether to shoot based on their position, angle to goal, pressure, and the tactical situation, and every shot taken SHALL be classified into a shot type (e.g. open-play foot strike, header, volley, one-on-one, cutback, rebound, penalty, direct free kick) that determines its baseline conversion quality independently of shot geometry.

#### Scenario: Penalty and direct free kick use fixed conversion bands
- **WHEN** a shot is taken from a penalty or a direct free kick
- **THEN** its expected-goals value is derived from that restart's own real-world-calibrated conversion band rather than from the shooter's distance and angle alone

#### Scenario: Header converts at a lower rate than an equivalent foot strike
- **WHEN** two shots are taken from the same distance and angle, one with the head and one with the foot in open play
- **THEN** the headed shot's expected-goals value is lower than the foot shot's

### Requirement: Tackling and duels for the ball
A defending player near an opponent in possession SHALL be able to attempt a tackle, with the outcome influenced by both players' relevant attributes and the contact severity. A player's willingness to engage SHALL account for a card already held, in every position group. Contact in a tackle or a collision SHALL carry an injury impulse to both players, scaled by the contact severity and each player's physical robustness, that feeds the in-match injury model.

A defender who engages a carrier SHALL do it in three phases, each a state of its own:
- **Closing:** he runs to the carrier.
- **Containing:** inside challenge range and goal-side of the carrier, he stays on him and decides when to challenge.
- **Challenging:** the challenge is made, and is decided once.

A defender SHALL be in the challenging state only to make a challenge. A defender who is containing SHALL go back to closing when the carrier gets goal-side of him or leaves him behind. A defender who is containing SHALL move no faster than a hard run, so a carrier at full stride can gain on him.

How a defender arrives SHALL count. A challenge made while he is still closing faster than the carrier is moving SHALL be less likely to win the ball, and more likely to be a foul, than the same challenge made after he has set himself.

#### Scenario: Tackle outcome depends on both players' attributes
- **WHEN** a defender attempts a tackle against an opponent in possession
- **THEN** the outcome of the duel is influenced by both the tackler's and the ball-carrier's relevant attributes and by the contact severity, rather than being decided by one side alone

#### Scenario: Reckless tackle can injure the carrier
- **WHEN** a reckless-severity tackle makes contact with the carrier
- **THEN** the carrier's in-match injury probability on that contact is higher than for a clean tackle, and may resolve into an injury of a severity decided in the match

#### Scenario: A defender contains before he challenges
- **WHEN** a defender reaches challenge range goal-side of a carrier and does not challenge at once
- **THEN** he is in the containing state, on the carrier's goal side, until he challenges, the carrier releases the ball or the carrier gets away from him

#### Scenario: The tackle state counts tackles
- **WHEN** a 100-match batch is played
- **THEN** the number of times defenders enter the challenging state equals the number of challenges they make, against 328 entries for 13.5 challenges a match before this change

#### Scenario: A carrier who gets goal-side is chased
- **WHEN** a carrier goes past the defender containing him
- **THEN** the defender is closing again, and the only challenge open to him is the stretched one a beaten defender makes

#### Scenario: Diving in
- **WHEN** two equal defenders challenge the same carrier, one arriving at a sprint and one who has been set for half a second
- **THEN** the defender who was set wins the ball more often and fouls less often

#### Scenario: Pace can beat a defender who is backing off
- **WHEN** a carrier with the pace and the carrying skill to run at full stride runs at a containing defender no quicker than he is
- **THEN** the carrier closes the distance between them faster than the defender can open it

### Requirement: Goalkeeper shot-stopping
The goalkeeper SHALL attempt to stop shots on target within his reach. He SHALL position himself, step or dive as the shot's trajectory and speed and his own distance from it require.

Each shot SHALL be adjudicated once, at the moment it reaches him. Its outcome SHALL decide what he does next:
- **Held:** the ball goes into his hands.
- **Parried, tipped out of play, spilled, deflected or beaten:** if he went to ground for it, he stays on the ground; if the ball is still alive in front of him and he is on his feet, he sets himself again.

Save success SHALL depend on the keeper's positioning, his reach, the shot's height as well as its width, and his relevant skills. It SHALL NOT depend on which state he happens to be in while the ball is in flight.

At a penalty he SHALL commit as the ball is struck: to one side on most kicks, and by staying central on a small share of them.

#### Scenario: Close-range shot inside reach is saveable
- **WHEN** a shot on target falls within the goalkeeper's positional reach envelope
- **THEN** the keeper has a save opportunity via the appropriate save action rather than automatically conceding

#### Scenario: One shot, one adjudication
- **WHEN** a shot reaches a keeper who has spent its flight first set and then diving
- **THEN** the shot's outcome is decided once, and no keeper is ever handed a shot that the adjudication has already let past him

#### Scenario: The outcome decides what he does next
- **WHEN** a keeper dives and parries a shot that stays in play
- **THEN** he is on the ground when the parry leaves him, and he does not hold the ball

#### Scenario: Height counts as well as width
- **WHEN** two shots cross a set keeper with the same sideways gap, one at chest height and one just under the bar
- **THEN** the shot under the bar is the less likely to be saved

#### Scenario: A penalty keeper sometimes stays central
- **WHEN** a seeded batch of matches produces at least 500 penalties
- **THEN** the keeper stays central on between 2% and 10% of them and dives to a side on the rest

### Requirement: Goalkeeper distribution and positioning
The goalkeeper SHALL distribute the ball after gaining possession by throwing it, kicking it or passing it, according to the tactical situation and the options available. He SHALL carry out the release he chose:
- a short release goes to a team-mate near him;
- a long release is kicked;
- with a counter-attack on, he releases within a second of having control;
- a ball he clears under pressure goes high, long and away from the middle of the pitch.

He SHALL adjust his position between the goal line and the edge of the ground he defends according to where play is. Across the goal he SHALL stand on the line that bisects the angle the two posts make at the ball.

#### Scenario: Keeper comes off the line to sweep
- **WHEN** the ball is played into space behind the defensive line with the goalkeeper closest to reach it
- **THEN** the goalkeeper can advance from the goal to intercept or clear it rather than remaining fixed on the line

#### Scenario: A short release is short
- **WHEN** a keeper chooses a short release with a free team-mate within 17.5 m of him
- **THEN** the ball goes to a team-mate within that distance, not to a midfielder on the halfway line

#### Scenario: A counter is released at once
- **WHEN** a keeper catches the ball with the opposition's back line pushed past the halfway line and a team-mate free ahead of the ball
- **THEN** he releases the ball within one second of taking control

#### Scenario: He covers the near post from a wide angle
- **WHEN** the ball is 16 m out and 16 m wide of the middle of his goal
- **THEN** he stands on the bisector of the angle formed by the ball and the two posts, which lies nearer his near post than the line from the ball to the middle of the goal

#### Scenario: A pressured clearance goes wide
- **WHEN** a keeper who may not handle the ball clears it with an opponent within 5 m of him
- **THEN** the ball leaves at a clearance's height and lands nearer the touchline than the middle of the pitch

### Requirement: Player fatigue and condition
Each player's on-pitch condition SHALL deplete based on the intensity and duration of their activity — higher-velocity and higher-intensity actions draining condition faster than low-intensity or stationary ones — and SHALL recover when the player is at low intensity, with the drain and recovery rates influenced by the player's stamina, natural fitness, and accumulated fatigue/jadedness from recent matches.

#### Scenario: Sprinting drains condition faster than jogging
- **WHEN** a player sustains near-maximum velocity rather than a jog
- **THEN** their condition depletes at a materially higher rate over the same duration

#### Scenario: Condition never drops below the match floor
- **WHEN** a player's condition would fall below the engine's minimum in-match floor from continued exertion
- **THEN** the value is held at that floor rather than continuing to fall

#### Scenario: Fatigue accelerates as the match progresses
- **WHEN** comparable activity intensity is sustained at the start of the match versus late in the match
- **THEN** the later exertion produces a greater condition drain and a smaller recovery credit than the same activity earlier

### Requirement: Effective performance under fatigue
A player's effective on-pitch capability (maximum speed and the outcome of skill-dependent actions) SHALL be reduced as their condition falls, so a tired player performs below a fresh player of identical underlying ability. Of two players at the same condition, the fitter SHALL lose no more of his capability than the less fit.

#### Scenario: Tired player's top speed is reduced
- **WHEN** a player's condition has dropped substantially below full
- **THEN** their maximum attainable speed for the remainder of that spell is lower than at full condition

#### Scenario: The fitter defender keeps more of his defending
- **WHEN** two defenders at the same condition below full differ only in stamina, natural fitness and match readiness
- **THEN** the fitter one holds the line, makes his recovery runs, presses and clears no worse than the other

### Requirement: In-match injury state
A player SHALL be able to sustain an in-match injury from either a contact impulse (tackle, collision) or accumulated load (fatigue, jadedness, age, natural fitness, environment), with a severity decided in the match — a knock the player shakes off, a hurt that lets them continue diminished, or a serious injury that ends their match. An injured player SHALL go down, recover no condition and be excluded from ball-chase and loose-ball contests; the referee SHALL stop play for a hurt or serious injury when the ball is not already dead, treatment SHALL take a time scaled by severity, and the player SHALL leave the injured state for a football reason (treatment complete, stretchered off, substituted) rather than a fixed timer. An injured player SHALL NOT be moved by a set-piece or restart shape and SHALL NOT take a restart; a restart he was due to take SHALL pass to a team-mate, and a ball he was carrying SHALL be released. A player who continues diminished SHALL carry a reduced effective ability for the rest of the match. The injury and its severity SHALL be handed to the player after the match through the player's own reaction method so the persistent injury the player carries out of the match is the one sustained in it, and the independent post-match roll SHALL run only when no in-match injury occurred.

#### Scenario: Injured player is excluded from active play
- **WHEN** a player is marked injured during the match
- **THEN** that player stops moving, recovers no condition, and is excluded from ball-chase and loose-ball contests until treatment resolves their status

#### Scenario: Serious injury stops play
- **WHEN** a player sustains a serious injury while the ball is in play
- **THEN** play is stopped, the dead time is measured for stoppage purposes, and play restarts with a drop ball

#### Scenario: Knock is shaken off
- **WHEN** a player sustains a knock
- **THEN** play is not stopped, the player is back in their role within a short treatment time, and no persistent injury is recorded after the match

#### Scenario: In-match injury persists after the match
- **WHEN** a player sustains a serious in-match injury
- **THEN** the player leaves the match carrying an injury whose type and length derive from that in-match severity, and no second independent injury roll is made for them

#### Scenario: An injured player stays down through a set piece
- **WHEN** a corner, a free kick or another shaped restart is set up while a player is injured
- **THEN** he does not move to a set-piece station

#### Scenario: An injured player does not take a restart
- **WHEN** a player is injured while he is the man due to take a restart
- **THEN** a team-mate takes the restart and the injured player does not play the ball

### Requirement: Per-player match statistics recording
For every player who featured in the match, the system SHALL record a stat line — attempted and completed actions (shots, passes, tackles, interceptions, dribbles, crosses, etc.), disciplinary events, goal involvement, and a computed match rating.

#### Scenario: Goalkeeper stat line includes shot-stopping detail
- **WHEN** a goalkeeper faces one or more shots on target during the match
- **THEN** the recorded stat line includes saves made, shots faced, and the expected-goals value of the shots faced, distinct from an outfield player's stat line

### Requirement: The winner of a decided aerial contest goes to meet the ball
When an aerial contest has been decided and its ball is still being delivered, the winner SHALL go to meet it,
whatever he was doing, short of an action already under way that cannot be abandoned. This SHALL hold for every role,
whatever the outcome (a header, a clearance or a ball hooked behind), and in open play and at set pieces alike.

- While the ball is in flight, the winner SHALL run to the point the delivery is aimed at. Once the ball is down in
  the heading band, he SHALL run to the ball.
- He SHALL NOT run to where the ball would have come down on the grass had nothing held it.
- He SHALL run at full effort.
- No positional hold, whether a set-piece station or the team's shape, SHALL keep him from the ball.
- His run SHALL end when the delivery ends: he gets within heading reach, the ball comes down to boot height, the
  delivery's deadline passes, or another player touches the ball.

What he does with the ball on arrival SHALL remain his role's decision.

#### Scenario: A defender who won a cross leaves his man for it
- **WHEN** a defender wins the aerial contest for a cross while marking an attacker away from where the ball is aimed
- **THEN** he runs to the aim point while the ball is in flight, and to the ball once it is held in the heading band

#### Scenario: The run does not overshoot the hold
- **WHEN** a won delivery's ball is held at its aim point, short of where its flight would have taken it to the grass
- **THEN** the winner runs to the held ball, not past it to that landing point

#### Scenario: A centre-back who won the corner is not held on his station
- **WHEN** a centre-back up for a corner wins the corner's aerial contest while the ball is still on its way from
  the flag
- **THEN** he runs to the delivery's aim point instead of being held at his corner station

#### Scenario: The run ends with the delivery
- **WHEN** the winner's delivery ends, because he plays it, it comes down to boot height, its deadline passes or
  another player touches it
- **THEN** his own state decides his movement again

#### Scenario: Only the winner is moved
- **WHEN** an aerial contest is decided
- **THEN** no player other than its winner is moved by this rule

### Requirement: A goalkeeper's state of mind shapes his tendencies
A goalkeeper's confidence and nerves SHALL shift how much he takes on, separately from how well he executes it:
- **Nervous:** he SHALL claim fewer crosses, come off his line less, parry more of the saves he could hold, and
  distribute long and safe more often.
- **Over-confident:** he SHALL claim more, sweep further from goal, hold more, and play out from the back more often.
- **Settled:** a keeper with neutral confidence and negligible nerves SHALL behave exactly as his attributes dictate.

#### Scenario: A nervous keeper stays at home
- **WHEN** the same keeper faces the same crosses once nervous and once settled
- **THEN** he attempts fewer claims when nervous

#### Scenario: A confident keeper comes for more
- **WHEN** the same keeper faces the same crosses once over-confident and once settled
- **THEN** he attempts more claims when over-confident

### Requirement: A goalkeeper's state of mind shapes his reliability
A goalkeeper's confidence and nerves SHALL change how often his actions fail:
- **Nerves and low confidence** SHALL raise errors of hesitation and handling: saveable shots let through, spilled
  into danger or fumbled, late or half-hearted claims, and rushed clearances or passes under pressure.
- **Over-confidence** SHALL carry the risk of ambition: the extra claims, sweeps and passes out of defence he takes
  on SHALL be priced by the same outcome rolls as any keeper's. It is not required to raise his share of failed
  claims, because the same confidence steadies his hands.

A failure that leads to an opposition shot or goal within the error window SHALL be recorded as his error leading to
a shot or goal.

Over a seeded batch representing seasons of matches between sides of equal level:
- the mean number of errors leading to a goal per keeper-season SHALL be between 0.6 and 1.5, and the 90th
  percentile between 2 and 5;
- goals per match and save percentage SHALL stay within their calibration bands.

#### Scenario: An unassured keeper makes more errors at equal attributes
- **WHEN** two keepers with identical attributes each play the same set of matches at the same standard, one assured
  at that standard and one well below it
- **THEN** the unassured keeper is charged with at least one and a half times as many errors leading to a shot

#### Scenario: Nerves cost claims
- **WHEN** the same keeper faces the same crosses once unassured and once assured
- **THEN** a larger share of his claims fail when he is unassured

#### Scenario: The keeper population keeps its level
- **WHEN** the seeded realism batch is run with keeper reliability in force
- **THEN** goals per match and save percentage stay within their calibration bands, and errors leading to a goal per
  keeper-season fall within the stated range

### Requirement: A goalkeeper is measured against the standard of football, not against the keepers on the pitch
The standard a goalkeeper's reach, read of the shot and handling are measured against SHALL be derived from the
standard of football in the match. It SHALL NOT depend on the attributes of either goalkeeper on the pitch, so that a
keeper is never measured against himself and never made better or worse by the keeper at the other end.

#### Scenario: The opposing keeper does not change his saves
- **WHEN** a keeper plays two otherwise identical matches in which only the opposing goalkeeper differs, one far
  better than the other
- **THEN** the standard his shot-stopping is measured against is the same in both matches

#### Scenario: A weak keeper keeps his whole gap
- **WHEN** a keeper whose goalkeeping attributes sit well below his division's typical keeper plays a season in that
  division
- **THEN** the standard his shot-stopping is measured against in each match does not move with his own attributes

### Requirement: The crosser prices the men in front of the delivery
A wide player SHALL NOT refuse to cross only because an opponent stands in front of him. He SHALL weigh each candidate delivery by its chance of getting past the men on its line, and that chance SHALL be the one the block contest rolls for that delivery. The type of delivery SHALL count: a ball that passes over a man's reach is not priced as blockable by him.

#### Scenario: A cross past a closer is still possible
- **WHEN** a crosser with a strong delivery on offer has a defender a stride in front of him on the delivery's line
- **THEN** he may still cross, with less appetite for it than he would have with nobody in front

#### Scenario: The ball over him is preferred to the ball through him
- **WHEN** two candidate deliveries are otherwise equal, but one passes over the closer's reach and the other runs through him
- **THEN** the delivery over him is valued higher

#### Scenario: The crosser reads the same chance the contest rolls
- **WHEN** a crosser weighs a delivery with opponents on its line
- **THEN** the chance of it being blocked that he weighs is the chance the block contest would roll for that delivery from the same positions

### Requirement: An outfield player's state of mind shapes his reliability
An outfield player's confidence and nerves SHALL change how often these actions fail:
- **Passing under the press:** a pass played with an opponent close SHALL more often be snatched off its target.
- **Receiving:** a touch SHALL more often get away from him.
- **Clearing:** a clearance, headed or kicked, SHALL more often be miscued. A miscued clearance falls short or
  sideways, and can go toward his own goal.
- **Challenging:** a challenge SHALL more often be mistimed into a foul. In his own penalty area, a mistimed
  challenge SHALL more often concede a penalty.
- **Finishing:** a shot at a good chance SHALL more often be snatched.
- **Marking:** when the man he is marking starts a run, he SHALL more often react late.

**Base rate.** With a neutral head, each of these actions SHALL still fail at a base rate. That rate is set by the
player's steadiness and the situation, so a settled veteran also makes occasional mistakes.

**Population.** Over a seeded batch of matches between sides of equal level:
- errors leading to a goal SHALL be between 0.10 and 0.35 per match;
- errors leading to a shot SHALL stay within their calibration band.

**Season ladder.** Over a seeded season ladder, among outfield players who start most of their side's matches:
- the mean number of errors leading to a goal per player-season SHALL be between 0.2 and 0.7;
- the 90th percentile SHALL be at most 3.

#### Scenario: An unassured outfield side makes more errors at equal attributes
- **WHEN** two sides with identical attributes play the same set of matches at the same standard, one side's
  outfield players assured at that standard and the other's well below it
- **THEN** the unassured outfield players are charged with at least one and a half times as many errors leading to
  a shot

#### Scenario: Nerves cost fouls
- **WHEN** the same defenders make the same challenges once nervous and once settled
- **THEN** a larger share of their challenges are fouls when they are nervous

#### Scenario: Nerves cost finishing
- **WHEN** the same forwards take shots at chances of the same quality once nervous and once settled
- **THEN** they score a smaller share of those chances when they are nervous

#### Scenario: A settled veteran still makes mistakes
- **WHEN** the seeded realism batch is run with every player settled
- **THEN** errors leading to a goal are at least 0.10 per match

### Requirement: A mistake is charged to the man whose execution failed
**Marking a giveaway.**
- A player SHALL be marked as having given the ball away when he loses it through his own failed execution:
  - a pass that went astray (snatched, miskicked or arriving well off its target) and ends with an opponent,
    whether picked off in flight or collected loose;
  - a miscontrol or heavy touch that an opponent collects;
  - a miscued clearance that an opponent collects.
- A player SHALL NOT be marked when his own execution did not fail:
  - an opponent's tackle or duel wins the ball from him;
  - an opponent reads and cuts out a pass that was going where it was meant to.

**Charging it.**
- The giver SHALL be charged one error leading to a shot when the opposition shoots both:
  - within the error window of the giveaway;
  - before his side regains the ball or play stops.
- Further shots in the same attack SHALL NOT charge him again.
- If that attack ends in a goal, the error SHALL be promoted to an error leading to a goal. This includes a goal from
  a rebound.
- The giveaway SHALL lapse when his side regains the ball or when play stops. A goal from a later phase SHALL NOT be
  charged to it.

**Goalkeepers.** A goalkeeper's spills and howlers SHALL be charged by the same rules.

#### Scenario: A rebound is charged once and promoted
- **WHEN** a defender's misplaced pass is collected by an opponent, whose shot is saved and the rebound scored within
  the error window
- **THEN** the defender is charged exactly one error leading to a shot and one error leading to a goal

#### Scenario: A misplaced pass collected loose is an error
- **WHEN** a midfielder's pass in his own half goes well off its target, an opponent collects it and shoots within
  the error window
- **THEN** the midfielder is charged with an error leading to a shot

#### Scenario: A pass read and cut out is not an error
- **WHEN** a pass arriving close to where it was aimed is cut out by an opponent, who shoots within the error window
- **THEN** the passer is not charged with an error

#### Scenario: A clean tackle is not an error
- **WHEN** a carrier who has made no failed touch or pass is tackled cleanly and the opponent shoots within the error
  window
- **THEN** the carrier is not charged with an error

#### Scenario: Regaining the ball clears the charge
- **WHEN** a player gives the ball away, his side wins it back, and then loses it to a clean tackle from which the
  opponent shoots within the original error window
- **THEN** the original giver is not charged with an error leading to a shot

### Requirement: A receiver meets the pass settled
A player going to take a pass played to him SHALL arrive at the point he takes it braking, the same way every other player is brought into the point a pass will be collected, so that the ball meets a man who has stopped running at it. A pass SHALL NOT be met by a receiver still running into its line at the moment of the touch when he had the time to settle.

#### Scenario: A pass he can reach in time
- **WHEN** a pass is played to a receiver who can be at the point he takes it before the ball
- **THEN** he arrives there braking and is settled when the ball reaches him
- **AND** the ball is not turned back along its own line at the touch

#### Scenario: The recorded track of receptions
- **WHEN** full matches are recorded and their ball tracks are read at the recorder's 30 ms cadence
- **THEN** direction reversals inside the 300 ms after a reception that no strike explains number at most one per minute of match time

### Requirement: A carried ball is read as going where its carrier goes
When a player reads the ball's motion — to judge whether a shot or a pass is coming, or where a rolling ball will be — a ball that another player is carrying, or that a goalkeeper holds, SHALL read as having no motion of its own: it goes where its carrier goes. A ball that is travelling on its own SHALL read at its own velocity.

#### Scenario: A dribbler running at goal
- **WHEN** an attacker carries the ball toward goal at a run
- **THEN** the goalkeeper and the defenders read the ball as carried, not as struck toward goal
- **AND** they react to the carrier, not to a shot

#### Scenario: The same ball struck
- **WHEN** the carrier shoots or passes
- **THEN** from that tick the ball reads at the velocity it was struck with

### Requirement: Chance volume does not rise with the level of football
Between two equally matched sides, the number of shots each side takes and the number of goals scored in a match SHALL NOT rise or fall with the level of football they play at, beyond the run-to-run noise of a divisional sweep. Before kickoff, every attribute of both squads SHALL be moved by the match's distance from the calibration division, so that every read in the engine — a decision, a contest, a stride — is against the football around the player and cancels between equal sides at every level, while a mismatch keeps its edge.

#### Scenario: The divisional sweep
- **WHEN** equal squads are played at every second level from 4 to 18, 90 matches a level, generated without the 1-20 clamp so the scale's saturation at the top and the bottom of the pyramid is not part of the comparison
- **THEN** the regression slope of goals per match on level is within two standard errors of zero
- **AND** the slope of shots per team on level is within two standard errors of zero

#### Scenario: Equal squads measured at both ends of the pyramid
- **WHEN** two equal squads at the mean of level 4 and two at the mean of level 18 are measured against the standard of their match
- **THEN** their measured attributes agree to within half a point
- **AND** the standard each pair brought is still reported as itself

#### Scenario: A mismatch keeps its edge
- **WHEN** a squad four points better than its opponent is measured against the standard of their match
- **THEN** it is still four points better

#### Scenario: Equal sides at the top and the bottom
- **WHEN** two equal level-18 sides and two equal level-4 sides each play 200 matches
- **THEN** shots per team and goals per match agree between the two batches within their noise

### Requirement: A first-time strike is weighed on how the ball arrived
A player who has just received the ball SHALL weigh a shot on how it reached him — its pace and how far he must turn it toward goal — against his technique, and that weight SHALL fade over the half second it takes to bring the ball under control. A soft lay-off is struck first time by anybody; a driven ball turned back toward goal is taken down first by all but the best technicians.

#### Scenario: A soft lay-off
- **WHEN** a forward in shooting range receives a ball arriving at walking pace
- **THEN** the way it arrived costs nothing in his decision to shoot

#### Scenario: A driven ball turned back toward goal
- **WHEN** a forward receives a driven ball coming from the direction of goal
- **THEN** striking it first time weighs more against him than a driven ball of the same pace running on toward goal
- **AND** more for an ordinary technician than for a gifted one

#### Scenario: Under control
- **WHEN** half a second has passed since his touch
- **THEN** the way the ball arrived no longer affects the decision

### Requirement: Dispossession reaction
A player who has just been dispossessed SHALL react within a bounded reaction time by counter-pressing the new carrier, chasing the loose ball, or committing a tactical foul, according to their attributes, the team's instruction and the counter-press window, rather than returning passively to their role's default state.

#### Scenario: Carrier loses the ball to a tackle
- **WHEN** a carrier is dispossessed by a tackle and the ball stays within pressing range
- **THEN** the dispossessed player immediately presses the new carrier or chases the loose ball, and his pressure on the new carrier is counted

### Requirement: Goalkeeper short distribution waits for an option
A keeper holding the ball who intends to distribute short SHALL wait for a team-mate to become an open option, for a time bounded by the permitted holding time and the coach's tempo instruction, before going long, rather than defaulting to a long clearance after a fixed fraction of a second.

#### Scenario: Short option arrives inside the hold window
- **WHEN** the keeper holds the ball and a team-mate becomes open within the permitted holding time
- **THEN** the keeper distributes short to that team-mate

#### Scenario: No option before the hold limit
- **WHEN** the keeper holds the ball and no short option becomes open before the permitted holding time expires
- **THEN** the keeper goes long before the holding limit is breached

### Requirement: State exits on match reasons
A player SHALL leave a resting state when their condition has recovered to a threshold or the ball demands them, SHALL release a forced pass or forced clearance because of pressure, option quality or danger rather than an elapsed tick count, and SHALL leave an off-ball holding pattern because of a positional trigger (ball zone, team phase, marker distance) rather than a fixed timer. Timers MAY remain only as a last-resort stall guard well beyond the football reason.

A forward SHALL have no resting state. A tired forward SHALL ease off by walking in his place, and SHALL NOT be stopped where he stands.

#### Scenario: Resting ends on recovery
- **WHEN** a player in a resting state recovers above the resting threshold while the ball is far from them
- **THEN** the player leaves the resting state at that moment, not at a fixed number of ticks after entering it

#### Scenario: A tired forward is not stopped
- **WHEN** a forward's condition falls below the level at which a forward used to stop and rest
- **THEN** he keeps his place in his side's shape at a walk, and takes every door a fresh forward takes

#### Scenario: Forced pass under pressure
- **WHEN** a midfielder carrying the ball is closed down with a pressing opponent within pressing range
- **THEN** the forced release fires because of the pressure, before any stall-guard timer would have

### Requirement: Trait biases consumed in live play
Each player's trait-driven movement bias (cutting inside, hugging the touchline, running in behind) and personality bias (dissent, caution once booked, diving in) SHALL influence the corresponding live decisions, so that two players of equal attributes but different traits behave differently in a match.

#### Scenario: Inverted winger cuts inside
- **WHEN** a wide player with a cut-inside trait receives the ball wide with space infield
- **THEN** their chosen run direction is biased infield relative to an identical player without the trait

### Requirement: A goalkeeper takes a goal kick with his feet
A goalkeeper taking a goal kick SHALL NOT handle the ball, whether it is waiting on the spot or has just been handed to him by the restart. Handling SHALL be illegal for a goal kick until it has been kicked (Law 16), wherever he is in his own area. While the restart waits he SHALL stand over the ball rather than trying to catch or gather it. When the restart hands him the ball he SHALL kick it, using the short-or-long decision made when he placed it: long off his run-up, or short with his feet from the spot. A goal kick SHALL never become a hand possession, so it SHALL never be thrown, never count as a keeper gather, and never draw the keeper-hold time-wasting delay or the hands limit.

#### Scenario: He stands over the waiting ball
- **WHEN** the keeper has reached a goal kick and the restart is still waiting
- **THEN** he stays over the ball and never enters a catch or a pick-up on it

#### Scenario: A short goal kick is played from the spot
- **WHEN** the restart hands the keeper a goal kick that he decided to play short
- **THEN** he passes it from the spot with his feet, with no hand possession and no throw

#### Scenario: A long goal kick is struck off the run-up
- **WHEN** the restart hands the keeper a goal kick that he decided to play long
- **THEN** he strikes it at the end of his run-up, with no hand possession

#### Scenario: Handling an unplayed goal kick is refused
- **WHEN** the keeper is in any state that would take the ball in his hands while it is a goal kick that has not yet been kicked
- **THEN** handling is judged illegal and he plays the ball with his feet instead

#### Scenario: The ball is handled again once the goal kick is played
- **WHEN** a goal kick has been kicked and the ball later comes back to the keeper inside his area by a route the Laws allow him to handle
- **THEN** handling is legal again, exactly as it is for any other ball

#### Scenario: No goal kick is thrown
- **WHEN** a batch of level-14 matches is played
- **THEN** no goal kick is released by a throw or from the keeper's hands, compared with 70% of goal kicks before this change

#### Scenario: A wasted goal kick is delayed once
- **WHEN** a side protecting a late lead takes a goal kick
- **THEN** the restart is held for the goal kick's time-wasting delay only, and no keeper-hold delay follows it

### Requirement: A decision resolves within its tick
A player's decision to change state SHALL be resolved, within the AI tick it is taken, to a state he can stay in. A state that would hand him on without doing anything SHALL NOT be occupied. When a chain of such hand-offs leads back to a state already visited in that tick, the player SHALL stay in the state he was in, and his time in that state SHALL keep running. A hand-off that carries an action (a pass, a shot, a tackle, a foul), a move into an action that cannot be abandoned, and the engine's out-of-band redirects SHALL take effect as they do today.

#### Scenario: A state that would refuse him is not entered
- **WHEN** a defender holding the line chooses to intercept a pass, and intercepting would hand him on to pressing and pressing would hand him back to the line
- **THEN** he stays holding the line, no transition is recorded and his time in that state keeps running

#### Scenario: A hop state is skipped
- **WHEN** a player chooses a state that hands him on, on the same evidence, to a second state that holds
- **THEN** he ends the tick in the second state, and the first is not recorded as visited

#### Scenario: An action is never swallowed
- **WHEN** a state reached during the resolution carries a pass, a shot, a tackle or a foul
- **THEN** that action is performed and the player ends the tick in the state it hands him to
- **AND** the state that performed the action is recorded as visited

#### Scenario: The population stops flapping
- **WHEN** 30 equal-strength level-14 matches are played
- **THEN** outfield players change state at most 45 times a minute on average, against 107.7 before this change
- **AND** at most 25% of state exits happen within one AI tick of entry, against 38.7% before this change
- **AND** no pair of off-ball states exchanges more than 3,000 round trips a match, against 17,000 for the largest pair before this change

### Requirement: A player's distance from his position is read from the team's current shape
A player deciding whether he has strayed too far from his position — to give up a man he is guarding, to abandon a press, or to walk back — SHALL measure that distance from the position his team's current shape gives him, and SHALL NOT measure it from where he stood at kickoff. His kickoff position MAY still decide which flank or slot in the formation is his.

#### Scenario: A midfielder who has moved with the block keeps his man
- **WHEN** a midfielder guarding an attacker has moved with his team's block more than 19 m from where he stood at kickoff, and is within his guarding range of where the shape now wants him
- **THEN** he keeps guarding the attacker and is not sent back to his position

#### Scenario: A forward presses high with his team pushed up
- **WHEN** a forward presses in the opposition half while his team's shape is pushed up behind him
- **THEN** the press is not abandoned because of how far he is from where he stood at kickoff

#### Scenario: A man genuinely out of position still goes back
- **WHEN** a midfielder following a man has gone beyond his guarding range from where the shape now wants him
- **THEN** he gives the man up and returns to his position

### Requirement: A player commits to an interception only for a ball he can contest
A player SHALL enter an interception only for a ball that nobody is carrying and that he can reach no later than the opposition. A player who cannot contest the ball SHALL keep doing what he was doing, and SHALL NOT be sent to press when no opponent has the ball, unless his team's plan has named him as its presser.

#### Scenario: A pass he cannot reach first
- **WHEN** an opposition pass travels near a defender who would lose the race to it
- **THEN** he does not enter an interception and keeps his position in the line

#### Scenario: A carried ball is not an interception
- **WHEN** an opponent carries the ball near a defender holding the line
- **THEN** he responds to the carrier, by pressing, covering or holding, and does not enter an interception

#### Scenario: A pass he can reach first
- **WHEN** an opposition pass travels toward a defender who would win the race to it
- **THEN** he goes to intercept it

### Requirement: An off-ball player does not pass through an on-ball state
A state whose purpose is deciding what to do with the ball SHALL NOT be entered by a player who does not have it, and a player SHALL decide whether to continue an off-ball run in the state that run belongs to.

#### Scenario: A forward who has created space
- **WHEN** a forward without the ball has created space while his team has possession
- **THEN** he carries on with his off-ball movement without entering the state that decides how to assist with the ball

#### Scenario: A forward receives the ball after creating space
- **WHEN** a forward who has been creating space receives the ball
- **THEN** he makes his on-ball decision as any other carrier does

#### Scenario: A defender without the ball
- **WHEN** a 100-match batch is played
- **THEN** a defender has the ball for at least 95% of the time he spends in the state that decides what to do with it, against 4% before this change

### Requirement: A player recovers after a committed challenge
A player who commits to a challenge and does not win the ball SHALL spend time recovering before he can run at full speed or challenge again. Recovery SHALL be brief after a block tackle and longer after a stretch or a slide, and SHALL be shortened by agility and balance. While recovering, he SHALL decelerate rather than run on, and he SHALL be excluded from ball-chase and loose-ball contests. A player who wins the ball SHALL NOT pay this cost. A challenge that fouls SHALL cost the challenger the recovery a miss of the same kind costs, when play is allowed to go on. A recovering player SHALL return to his role's normal play when recovery ends.

#### Scenario: A missed slide
- **WHEN** a defender slides at a carrier and misses
- **THEN** he cannot run at full speed or challenge again for the slide's recovery time, and the carrier gets away from him

#### Scenario: A missed block tackle
- **WHEN** a defender on his feet misses a block tackle
- **THEN** he loses a shorter recovery time than he would after a slide, and is then free to turn and chase

#### Scenario: A won tackle
- **WHEN** a player wins the ball with his challenge
- **THEN** he plays on with the ball without any recovery time

#### Scenario: An agile player gets up sooner
- **WHEN** two players with different agility and balance miss the same kind of challenge
- **THEN** the more agile and balanced player recovers sooner

#### Scenario: The population after a missed challenge
- **WHEN** 100 equal-strength level-14 matches are played
- **THEN** mean speed in the first 0.5 s after a missed challenge is below the outfield mean speed, against 4.63 m/s after an attempt and 3.73 m/s on average before this change

#### Scenario: A foul the referee waves on
- **WHEN** a defender's block tackle fouls the carrier and the referee plays advantage
- **THEN** the defender recovers for as long as he would after missing a block tackle, and does not run on at once

### Requirement: Engagement cadence does not depend on state changes
How often a marking player's restraint foul is decided, and how jadedness accumulates, SHALL depend on the time he actually spends in contact with his man or on the pitch, and SHALL NOT depend on how many times his state changes. A state change SHALL NOT reset or delay either cadence.

#### Scenario: The same contact with and without a state change
- **WHEN** two markers spend the same time in contact range of their men, one in a single state and one passing through another state
- **THEN** both have the same exposure to a restraint foul

#### Scenario: Reduced state churn leaves the foul rate in band
- **WHEN** a 300-match batch is played after state churn has been reduced
- **THEN** fouls, yellow cards, penalties and penalty goals per match are within their realism bands

### Requirement: Every player state occurs in real matches or is retired
Every state a player can occupy SHALL be entered in a batch of 400 matches covering the formations the state belongs to, or SHALL be retired. A state SHALL NOT remain whose entry condition cannot be met within the condition range players actually reach in a match. Midfielders SHALL be able to walk through the same kind of football situation in which forwards and defenders walk.

#### Scenario: A midfielder with nothing to run for walks
- **WHEN** a fresh midfielder is in position, the ball is far away, no opponent threatens him and his team has it
- **THEN** he walks, whichever off-ball state he was deciding from

#### Scenario: A tired defender eases off when he can
- **WHEN** a defender is tired and the ball is far enough away for him to stand down
- **THEN** he drops to a walk in his place in the line sooner than a fresh defender would, and is alert again as soon as the ball or a threat demands it

#### Scenario: A tired defender does not leave the line
- **WHEN** a defender's condition falls during a match
- **THEN** no state stops him where he stands while the line moves away from him

#### Scenario: The census names any state still missing
- **WHEN** the 400-match batch is played
- **THEN** the census names every state never entered, and each named state is one documented as specific to a formation that batch did not use

### Requirement: A goalkeeper's readiness follows the play
The goalkeeper SHALL be in one of three phases:
- **Relaxed:** his side has the ball far from his goal.
- **Alert:** the opposition has the ball, or the ball is coming toward his goal.
- **Set:** a strike is imminent.

The phase SHALL follow possession and the ball's distance from his goal. Entering and leaving a phase SHALL use different thresholds, so that the phase does not flicker. The phase SHALL change how he holds himself and how fast he moves. It SHALL NOT change the point he rests on.

#### Scenario: Relaxed while his side attacks
- **WHEN** his side has the ball in the opponents' half
- **THEN** the keeper is relaxed, at his rest point, moving no faster than a walk unless he is out of position

#### Scenario: Alert when the ball is lost
- **WHEN** the opposition wins the ball within 60 m of his goal
- **THEN** he is alert from that moment, whatever state he was in

#### Scenario: The phase does not flicker
- **WHEN** a 100-match batch is played
- **THEN** a keeper changes between relaxed and alert fewer than 4 times a minute, and fewer than 10% of those changes reverse within 300 ms

### Requirement: Every goalkeeper state reads a situation the same way
A goalkeeper not in possession SHALL make the same decision about the same situation whichever state he is in. In particular:
- he gathers a loose ball he is favourite for and may handle;
- he plays a loose ball he may not handle with his feet;
- he does not chase a ball he has just played himself;
- he attacks a high ball he can claim;
- he meets a carrier who is through on goal;
- he sets himself for a struck shot;
- a ball moving toward him from his own side's play does not set him for a save.

When he already owns the ball, he SHALL always use the same decision between picking it up and playing it.

#### Scenario: A cross while he is repositioning
- **WHEN** a cross he can claim comes in while the keeper is recovering his position
- **THEN** he comes for it on the same terms as he would from his rest point

#### Scenario: A loose ball an opponent is nearer
- **WHEN** a loose ball lies two metres from the keeper and one metre from an opponent
- **THEN** he does not set off to gather it, whichever state he is in

#### Scenario: A back-pass does not set him
- **WHEN** a team-mate plays the ball back toward him at pace
- **THEN** he does not set himself for a save

#### Scenario: A ball at his feet
- **WHEN** a keeper wins the ball at his feet inside his area with an opponent closing
- **THEN** he chooses between picking it up and playing it with his feet by the same rule, whichever state won him the ball

### Requirement: A goalkeeper meets a man through on goal with one plan
An opponent may carry the ball toward goal with no defender goal-side of him. When that happens the goalkeeper SHALL:
- close him down at running pace to a stand-off point on the line from his goal to the ball;
- hold that point, without sprinting at the ball;
- go to ground at the ball only once it is inside his own spread;
- set himself for the shot if it is struck first.

This SHALL hold whichever state he was in when the one-on-one began. A carrier who is covered by a defender SHALL be left to the defence.

#### Scenario: The same one-on-one from two starting states
- **WHEN** the same breakaway begins once with the keeper at his rest point and once while he is recovering his position
- **THEN** he closes down to the same stand-off point and holds it in both

#### Scenario: He does not sprint at a man in control
- **WHEN** a carrier through on goal keeps the ball under close control
- **THEN** the keeper arrives at his stand-off point and stands there, rather than running through it at the ball

#### Scenario: A covered carrier is the defence's
- **WHEN** a defender is goal-side of the carrier and in his path
- **THEN** the keeper does not come out to meet the carrier

### Requirement: A goalkeeper on the ground gets up before he can save again
A goalkeeper who goes to ground, whether in a dive, a smother or a block, SHALL stay down for a time that his agility and balance shorten. While he is down he SHALL:
- move no faster than a scramble;
- reach only what he can reach lying down;
- not dive again.

A keeper on the ground with the ball in his hands SHALL keep it, and get up when he chooses to. A keeper who does not have the ball SHALL set himself again once he is up.

#### Scenario: A rebound finds him down
- **WHEN** a keeper parries a shot and an opponent shoots from the rebound half a second later
- **THEN** the keeper is still down. He can stop the second shot only with what he reaches lying down. He does not dive.

#### Scenario: He does not slide along the ground
- **WHEN** a keeper lands from a dive without the ball
- **THEN** he does not move faster than 1.5 m/s until he is back on his feet

#### Scenario: An agile keeper is up sooner
- **WHEN** two keepers with different agility and balance land from the same dive
- **THEN** the more agile and balanced keeper is back on his feet sooner

#### Scenario: Time down across the population
- **WHEN** a 100-match batch is played
- **THEN** the mean time a keeper spends down after a dive without the ball lies between 0.6 s and 1.2 s

### Requirement: A cross is claimed, punched or lost once
When the goalkeeper comes for a high ball:
- He SHALL choose whether to catch it or punch it as he comes. The choice SHALL depend on the traffic, the pace of the ball, and his own handling and nerve.
- The aerial contest SHALL decide once whether that action succeeds or fails. A flap is one of its outcomes.
- The height he commits to and the height the contest lets him play the ball at SHALL be the same.
- After a missed punch or a flap he SHALL land on his feet. He SHALL NOT be sent into a dive.
- A punch SHALL send the ball high, long and away from the middle of his goal.

#### Scenario: One decision per cross
- **WHEN** a keeper comes for a cross and leaves the ground for it
- **THEN** whether he catches, punches or loses the ball is decided once, and no later roll can change it

#### Scenario: He can reach what he came for
- **WHEN** the keeper commits to a high ball as within his reach
- **THEN** the contest treats it as within his reach as well

#### Scenario: A flap does not become a dive
- **WHEN** a keeper misses a punch at a cross
- **THEN** he lands on his feet and does not go into a dive

#### Scenario: A punch clears the middle
- **WHEN** a keeper punches a cross that arrives in front of the middle of his goal
- **THEN** the ball leaves away from the centre of the pitch, and in most cases comes down wider than the edge of the six-yard box

### Requirement: A goalkeeper is set when the shot is struck
While no shot is in flight and a strike is imminent, the goalkeeper's set stance SHALL move no faster than a set keeper's shuffle. Sprinting is for coming off his line, closing down a one-on-one and recovering his position. A keeper still moving when a shot is struck SHALL react later than a keeper who was set.

#### Scenario: Set, not sprinting
- **WHEN** an opponent on the ball changes the angle 20 m from goal and no shot is in flight
- **THEN** the keeper adjusts across his goal at no more than 4 m/s

#### Scenario: Caught moving
- **WHEN** the same shot is struck once at a set keeper and once at the same keeper running across his goal
- **THEN** the running keeper starts his save later

### Requirement: A replacement goalkeeper plays as a goalkeeper
A player who takes over in goal SHALL play the rest of the match with the goalkeeper's decisions and his own attributes, from the moment he is stationed in goal. This covers both a substitute keeper and an outfielder going in goal after the keeper is sent off or injured.

#### Scenario: A substitute keeper after a red card
- **WHEN** the keeper is sent off and the bench keeper comes on in place of an outfielder
- **THEN** from his first tick on the pitch he makes goalkeeper decisions in goal

#### Scenario: An emergency keeper
- **WHEN** an outfielder goes in goal with no substitutions left
- **THEN** he makes goalkeeper decisions with his own goalkeeping attributes and is never pulled out of goal to a set-piece station

### Requirement: A defender's readiness follows the play
A defender holding his place in the shape SHALL be in one of two phases:
- **Relaxed:** play is far away. His side has the ball well upfield of the back line, or the opposition has it far from his goal.
- **Alert:** the opposition has the ball in reach of the line, or the ball is loose or close to the line.

The phase SHALL follow possession and the ball's distance from the back line. Entering and leaving a phase SHALL use different thresholds, so that the phase does not flicker. A tired defender SHALL be relaxed at a shorter distance than a fresh one. The phase SHALL change how he holds himself and how fast he moves. It SHALL NOT change the place he holds.

#### Scenario: Relaxed while his side attacks
- **WHEN** his side has settled possession more than 30 m upfield of the back line
- **THEN** a defender on his slot is relaxed

#### Scenario: Alert when the ball is lost
- **WHEN** the opposition wins the ball within 60 m of his goal
- **THEN** he is alert from that moment, whatever state he was in

#### Scenario: The same place in both phases
- **WHEN** a defender changes from relaxed to alert with nothing else changed
- **THEN** the point he steers at is the same before and after

#### Scenario: The phase does not flicker
- **WHEN** a 100-match batch is played
- **THEN** a defender changes between relaxed and alert fewer than 4 times a minute, and fewer than 10% of those changes reverse within 300 ms

### Requirement: Every defender state reads a situation the same way
A defender without the ball SHALL make the same decision about the same situation whichever state he is in. In particular:
- he plays a ball that is his;
- he goes up for a header he has won;
- he chases a loose ball only when his side's election gives it to him;
- he engages a carrier when he is the man for him, and only then;
- he neither marks nor covers while his own side has the ball;
- he gets back to the line when he is out of it and has no job that keeps him out.

No state SHALL hand a defender to an idle state for the decision to be taken again there.

#### Scenario: A carrier in the area
- **WHEN** an opponent carries the ball in the defending side's penalty area and a defender is one of the two nearest to him
- **THEN** that defender engages him, whether he was walking, recovering, marking or holding the line

#### Scenario: A recovering defender challenges
- **WHEN** a defender running back to the line becomes the man for the carrier
- **THEN** he engages the carrier from his recovery run, and does not have to reach the line first

#### Scenario: Nobody marks while his own side has the ball
- **WHEN** a side wins the ball and holds it
- **THEN** none of its defenders is still marking or covering after the next AI tick

#### Scenario: One answer from every state
- **WHEN** the same situation is put to a defender in each state he can hold without the ball
- **THEN** each state's decision is the same one

#### Scenario: A chase the election will refuse is not started
- **WHEN** a 30-match batch is played
- **THEN** fewer than 20% of the loose-ball chases a defender's own decision starts are reversed within ten AI ticks, against 96% before this change

#### Scenario: No hub
- **WHEN** a 30-match batch is played
- **THEN** fewer than 1% of a defender's state changes pass through the state he starts a match in, against 5% before this change, and a defender changes state at most 20 times a minute on average, against 25

### Requirement: A defender marks with one model
A defender who marks SHALL do so in one state, with one way of choosing his man and one way of deciding how close to stand:
- his man is the one his side's assignment gave him;
- with no man assigned, it is the most dangerous opponent in his zone that the assignment has given to nobody;
- he stands goal-side of his man at a distance that tightens continuously as the man gets nearer the goal;
- outside his own penalty area he does not follow his man beyond the bound his place in the line allows; inside it, the man comes first.

#### Scenario: A man given to another defender is left to him
- **WHEN** the assignment has given an attacker to one defender and a second defender has no man
- **THEN** the second defender does not choose that attacker while another opponent in his zone is unmarked

#### Scenario: Tight near goal, off in midfield
- **WHEN** a defender marks a man 20 m from the goal and, at another time, a man 45 m from it
- **THEN** the distance he aims to stand off the first is no more than 1.75 m, and off the second no more than 2.5 m and no less than off the first

#### Scenario: A marker stays in the unit
- **WHEN** a defender's man runs across the pitch outside the penalty area
- **THEN** the defender follows him no further than his place in the line allows, and the man becomes the next defender's

#### Scenario: Marking is no worse for being one state
- **WHEN** a 300-match batch is played
- **THEN** a marker sits no further from his man on average than the 4.4 m before this change, and no more than 33% of attackers in the defending third have nobody within 3 m

### Requirement: A defender out of the line gets back at the pace the play demands
A defender who is out of the line and has no job that keeps him out SHALL get back to his place in it. With the opposition on the ball, or the ball loose, it SHALL be a recovery run, and he SHALL engage the carrier on the way if he becomes the man for him. With his own side on the ball, or the ball dead, he SHALL get back at no more than a jog.

#### Scenario: A recovery run with the ball in behind
- **WHEN** the opposition has the ball goal-side of a defender who is out of the line
- **THEN** he runs back toward his place at his full effort

#### Scenario: Getting back with his side on the ball
- **WHEN** a centre-back is returning from his side's corner while his side still has the ball
- **THEN** he moves no faster than a jog

### Requirement: A header a defender has won is played by him, once
A defender who has won a high ball SHALL go up for it in his heading state and play it himself. That state SHALL carry the header out and SHALL NOT decide the contest again. A header that sends the ball away from his own goal SHALL be recorded as a clearance and never as a shot.

#### Scenario: The winner goes up
- **WHEN** a defender wins the contest for a cross into his own area
- **THEN** he is in his heading state, off the ground, when the ball is headed

#### Scenario: No second roll
- **WHEN** a defender who has been awarded a high ball reaches it
- **THEN** he heads it, and nothing rolls again for whether he does

#### Scenario: A clearing header is not a shot
- **WHEN** a defender heads the ball away from his own goal
- **THEN** no shot is recorded for him, and a clearance is

#### Scenario: A ball awarded to another man is left
- **WHEN** a high ball has been awarded to one player and another, in any state, is within heading reach of it
- **THEN** only the player it was awarded to heads it

### Requirement: A defender moves at the pace the play asks for
While the play waits on his side, that is while his own side has the ball or the ball is dead, a defender's pace SHALL follow how soon the play can come back at the space he guards:
- when he can be in his place at a walk before the play can reach it, he walks;
- as that time shrinks he jogs, then runs;
- while the ball is dead he walks, unless he cannot be in his place before the restart is taken.

With the opposition on the ball he SHALL move at the effort his job asks, and a ball goal-side of him SHALL send him to cover or recover at his full effort.

A marker in position SHALL move with his man: at his man's pace, faster only by what closing the gap needs. The rule that recalls a defender to his place and the rule that keeps him goal-side SHALL run at the same pace as his own movement would, for the same situation.

#### Scenario: Play far away
- **WHEN** a defender is on his slot and the ball is 60 m away with his own side on it
- **THEN** he moves no faster than 2 m/s

#### Scenario: The ball in behind
- **WHEN** an opponent runs onto a ball goal-side of a defender
- **THEN** the defender is sent to cover or recover, at his full effort, whichever state he was in

#### Scenario: A marker of a walking man walks
- **WHEN** a defender is in position on a man who is walking
- **THEN** the defender moves at a walk

#### Scenario: A dead ball
- **WHEN** the ball is out of play and a defender is within 10 m of his place
- **THEN** he walks to it

#### Scenario: A defender no longer runs all match
- **WHEN** a 300-match batch is played
- **THEN** a defender covers at most 17.5 km per 90 minutes, against 20 km before this change, and is at or below 2 m/s for at least 30% of live play
- **AND** the batch reports both against the real 9.5 to 12 km and 50% to 75%

#### Scenario: Walking defenders do not open the game
- **WHEN** the same batch is played
- **THEN** goals, shots and expected goals per match stay inside their realism bands, and defenders are goal-side of the ball at a shot no less often than before

### Requirement: A defender keeps his own place behind the ball in possession
While his side has the ball, a defender who is not on it and has no named run SHALL hold his own place in the back line's shape as that shape moves up behind the ball. Defenders SHALL NOT steer at one shared point, and the deepest defender SHALL NOT be sent back alone. When the ball is lost, he SHALL react from where he is, by the same read as any defender.

#### Scenario: Each defender on his own slot
- **WHEN** a side has the ball in the attacking third and its back four is squeezing up
- **THEN** each of the four is nearer his own place in the line than any other defender's

#### Scenario: The deepest man holds the line
- **WHEN** the back line is moving up with the ball
- **THEN** its deepest defender moves up with it, and is not running back toward his own goal

#### Scenario: Losing the ball
- **WHEN** a side loses the ball with its back line squeezed up
- **THEN** each defender takes the job the assignment gives him, or gets back to the line, and none is sent to cover by default

### Requirement: A midfielder's readiness follows the play
A midfielder holding his place in his side's shape SHALL be in one of two phases:
- **Relaxed:** play is far from his place.
- **Alert:** the ball is near his place, or the opposition has it within reach of his place, or it is loose near it.

The phase SHALL follow possession and the ball's distance from his place. Entering and leaving a phase SHALL use different thresholds, so that the phase does not flicker. A tired midfielder SHALL be relaxed at a shorter distance than a fresh one. The phase SHALL change how he holds himself and how fast he moves. It SHALL NOT change the place he holds.

No state SHALL stop a midfielder where he stands while his side's shape moves away from him.

#### Scenario: Relaxed while play is far away
- **WHEN** his side has settled possession with the ball more than 30 m from his place, and the attack has given him no job
- **THEN** a midfielder on his place is relaxed

#### Scenario: Alert when the ball is lost near him
- **WHEN** the opposition wins the ball within 40 m of his place
- **THEN** he is alert from that moment, whatever state he was in

#### Scenario: The same place in both phases
- **WHEN** a midfielder changes from relaxed to alert with nothing else changed
- **THEN** the point he steers at is the same before and after

#### Scenario: The phase does not flicker
- **WHEN** a 100-match batch is played
- **THEN** a midfielder changes between relaxed and alert fewer than 6 times a minute, and fewer than 10% of those changes reverse within 300 ms

#### Scenario: A tired midfielder eases off in his place
- **WHEN** a midfielder's condition has fallen and the ball is far enough away for him to stand down
- **THEN** he drops to a walk in his place sooner than a fresh midfielder would, and is alert again as soon as the ball or a threat demands it

#### Scenario: A midfielder has somewhere to stand
- **WHEN** a 100-match batch is played
- **THEN** the batch reports the share of the match a midfielder spends holding his place, relaxed or alert, against 0.1% before this change

### Requirement: Every midfielder state reads a situation the same way
A midfielder without the ball SHALL make the same decision about the same situation whichever state he is in. In particular:
- he plays a ball that is his;
- he goes up for a header he has won;
- he chases a loose ball only when his side's election gives it to him;
- he engages a carrier when he is the man for him, and only then;
- he marks the man his side's assignment gave him while the opposition has the ball, and nobody while his own side has it;
- he gets back to his place when he is out of it and has no job that keeps him out;
- with his own side on the ball, he does the job the attack's assignment gave him.

No state SHALL hand a midfielder to an idle state, or to the state he carries the ball in, for the decision to be taken again there. A midfielder SHALL NOT leave an off-ball state because a fixed time has passed.

#### Scenario: A carrier near a midfielder who is not the man for him
- **WHEN** an opponent carries the ball 8 m from a midfielder who is not the man for him
- **THEN** the midfielder does not close him, whether he was walking, holding his place, marking, recovering or returning from an attack

#### Scenario: A carrier near the man for him
- **WHEN** a midfielder is the man for a carrier
- **THEN** he engages him from whichever off-ball state he was in, a recovery run included

#### Scenario: The ball is lost
- **WHEN** a side loses the ball while its midfielders are making runs, showing for it and holding the touchlines
- **THEN** on the next AI tick each of them is doing what the read gives him against the opposition's possession
- **AND** none of them carries on with his attacking job until a time has run out, and they are not all sent to close the ball

#### Scenario: One answer from every state
- **WHEN** the same situation is put to a midfielder in each state he can hold without the ball
- **THEN** each state's decision is the same one

#### Scenario: A chase the election will refuse is not started
- **WHEN** a 30-match batch is played
- **THEN** fewer than 20% of the loose-ball chases a midfielder's own decision starts are reversed within ten AI ticks, against 98% before this change

#### Scenario: The state that carries the ball holds the man on the ball
- **WHEN** a 100-match batch is played
- **THEN** a midfielder has the ball for at least 95% of the time he spends in the state that decides what to do with it, against 21% before this change

#### Scenario: Churn is reported by its cause
- **WHEN** a 30-match batch is played
- **THEN** the batch reports how many times a minute a midfielder changes state, and apart from them the changes that follow a change of the job his side's defensive plan gives him

### Requirement: A midfielder engages a carrier by closing, containing and challenging
A midfielder who engages a carrier SHALL do it in three phases, each a state of its own:
- **Closing:** he runs to the carrier.
- **Containing:** inside challenge range and goal-side of the carrier, he stays on him and decides when to challenge.
- **Challenging:** the challenge is made, and is decided once.

A midfielder SHALL be in the challenging state only to make a challenge. While containing he SHALL stand off the carrier on the carrier's goal side; he SHALL NOT run at the ball. A midfielder who is containing SHALL go back to closing when the carrier gets goal-side of him or leaves him behind, and SHALL move no faster than a hard run, so a carrier at full stride can gain on him.

How a midfielder arrives SHALL count. A challenge made while he is still closing faster than the carrier is moving SHALL be less likely to win the ball, and more likely to be a foul, than the same challenge made after he has set himself.

A midfielder the carrier has gone past SHALL be able to make the stretched challenge from behind that a beaten defender makes, with its longer recovery.

A ball that comes loose near the duel SHALL be won through his side's loose-ball election, as from any other state.

#### Scenario: A midfielder contains before he challenges
- **WHEN** a midfielder reaches challenge range goal-side of a carrier and does not challenge at once
- **THEN** he is in the containing state, on the carrier's goal side, until he challenges, the carrier releases the ball or the carrier gets away from him

#### Scenario: He does not run through the man
- **WHEN** a midfielder is containing a carrier who is standing over the ball
- **THEN** the midfielder holds his distance on the goal side and is not running at the ball

#### Scenario: The tackle state counts tackles
- **WHEN** a 100-match batch is played
- **THEN** the number of times midfielders enter the challenging state equals the number of challenges they make, against 521 entries for 23 challenges a match before this change

#### Scenario: A carrier who gets goal-side is chased
- **WHEN** a carrier goes past the midfielder containing him
- **THEN** the midfielder is closing again, and the only challenge open to him is the stretched one from behind

#### Scenario: Diving in
- **WHEN** two equal midfielders challenge the same carrier, one arriving at a sprint and one who has been set for half a second
- **THEN** the midfielder who was set wins the ball more often and fouls less often

#### Scenario: Pace can beat a midfielder who is backing off
- **WHEN** a carrier with the pace and the carrying skill to run at full stride runs at a containing midfielder no quicker than he is
- **THEN** the carrier closes the distance between them faster than the midfielder can open it

#### Scenario: A loose ball beside the duel
- **WHEN** the ball comes loose within reach of a midfielder who was containing its carrier
- **THEN** he goes for it only if his side's election gives it to him, and no rule of his challenging state takes it for him

#### Scenario: Closing wins the ball more often
- **WHEN** a 300-match batch is played
- **THEN** successful pressures per match are no fewer than the 13.0 before this change, and a midfielder's pressures per match are fewer than the 30.5 before it

### Requirement: A midfielder marks with one model
A midfielder who marks SHALL do so in one state, with one way of choosing his man and one way of deciding how close to stand:
- his man is the one his side's assignment gave him;
- with no man assigned, it is the most dangerous opponent in his zone that the assignment has given to nobody;
- he stands goal-side of his man at the distance a defender stands off a man in the same place, which tightens continuously as the man gets nearer the goal;
- outside his own penalty area he does not follow his man beyond the bound his place in the shape allows; inside it, the man comes first.

A midfielder SHALL NOT give his man up because of how near the two of them are to the edge of the pitch. He SHALL be able to commit a holding foul on his man only while he is marking him and the opposition has the ball.

#### Scenario: A man given to another player is left to him
- **WHEN** the assignment has given an attacker to one player and a midfielder has no man
- **THEN** the midfielder does not choose that attacker while another opponent in his zone is unmarked

#### Scenario: Tight near goal, off in midfield
- **WHEN** a midfielder marks a man 20 m from the goal and, at another time, a man 50 m from it
- **THEN** the distance he aims to stand off the first is no more than 1.75 m, and off the second no more than 2.5 m and no less than off the first

#### Scenario: A winger on the touchline is still marked
- **WHEN** the man a midfielder marks is standing a metre inside the touchline, within the bound the midfielder's place allows
- **THEN** the midfielder keeps marking him

#### Scenario: No shirt-pull once his side has the ball
- **WHEN** a midfielder's side wins the ball while he is beside the man he was marking
- **THEN** he commits no holding foul on that man

#### Scenario: Marking is no worse for it
- **WHEN** a 300-match batch is played
- **THEN** the man marking a midfielder sits no further from him on average than the 6.4 m before this change, and fouls per match stay inside their realism band

### Requirement: A midfielder out of his place gets back at the pace the play demands
A midfielder who is out of his place in the shape and has no job that keeps him out SHALL get back to it. With the opposition on the ball, or the ball loose, it SHALL be a recovery run in a state of its own, and he SHALL engage the carrier on the way if he becomes the man for him. With his own side on the ball, or the ball dead, he SHALL get back at no more than a jog.

#### Scenario: A recovery run with the ball behind him
- **WHEN** the opposition has the ball goal-side of a midfielder who is out of his place
- **THEN** he runs back toward his place at his full effort

#### Scenario: Getting back with his side on the ball
- **WHEN** a midfielder is returning from the opposition's area while his side still has the ball
- **THEN** he moves no faster than a jog

#### Scenario: The recovery run is the opposition's possession
- **WHEN** a 100-match batch is played
- **THEN** the opposition has the ball, or it is loose, on at least 95% of the ticks a midfielder spends in the recovery-run state

### Requirement: A header a midfielder has won is played by him, once
A midfielder who has won a high ball SHALL go up for it in his heading state and play it himself. That state SHALL carry the header out and SHALL NOT decide the contest again. What he does with it SHALL follow where he is: at goal from a genuine chance, clear from his own third, and down into the space ahead of him otherwise. A header that sends the ball away from his own goal SHALL be recorded as a clearance. A header that is not aimed at the goal SHALL NOT be recorded as a shot.

#### Scenario: The winner goes up
- **WHEN** a midfielder wins the contest for a ball dropping in midfield
- **THEN** he is in his heading state, off the ground, when the ball is headed

#### Scenario: No second roll
- **WHEN** a midfielder who has been awarded a high ball reaches it
- **THEN** he heads it, and nothing rolls again for whether he does

#### Scenario: A clearing header is not a shot
- **WHEN** a midfielder heads the ball away from his own goal inside his own third
- **THEN** no shot is recorded for him, and a clearance is

#### Scenario: A knock-down is not a shot
- **WHEN** a midfielder heads a ball down into the space ahead of him in the middle third
- **THEN** no shot is recorded for him

#### Scenario: A ball awarded to another man is left
- **WHEN** a high ball has been awarded to one player and a midfielder, in any state, is within heading reach of it
- **THEN** the midfielder does not head it

#### Scenario: Midfield headers are played by midfielders
- **WHEN** a 100-match batch is played
- **THEN** at least 90% of the headers midfielders play are played by a midfielder in his heading state, off the ground, and none of them is decided by a roll of that state
- **AND** a high ball only one side can reach, which a midfielder claims, is a reception and not a header: the batch reports it apart

### Requirement: A midfielder moves at the pace the play asks for
A midfielder's pace SHALL follow how soon the play can reach the space he guards:
- when he can be in his place at a walk before the play can reach it, he walks;
- as that time shrinks he jogs, then runs;
- with the opposition on the ball goal-side of him, he runs at his full effort;
- with his own side on the ball and no run given to him, he keeps pace with his place as the shape moves, and no more;
- while the ball is dead he walks, unless he cannot be in his place before the restart is taken.

A marker SHALL move with his man: at his man's pace when he is in position, faster only by what closing the gap needs. The rule that recalls a midfielder to his place SHALL run at the same pace as his own movement would, for the same situation.

#### Scenario: Play far away
- **WHEN** a midfielder is on his place and the ball is 50 m away with his own side on it
- **THEN** he moves no faster than 2 m/s

#### Scenario: The ball in behind
- **WHEN** the opposition breaks past a midfielder with the ball
- **THEN** he runs back at his full effort, whichever state he is in

#### Scenario: A marker of a walking man walks
- **WHEN** a midfielder is in position on a man who is walking
- **THEN** the midfielder moves at a walk

#### Scenario: A dead ball
- **WHEN** the ball is out of play and a midfielder is within 10 m of his place
- **THEN** he walks to it

#### Scenario: A midfielder's running is reported
- **WHEN** a 300-match batch is played
- **THEN** the batch reports the distance a midfielder covers per 90 minutes and the share of live play he spends at or below 2 m/s, against 22 km before this change and against the real 10.5 to 12.5 km and 45% to 70%

#### Scenario: Walking midfielders do not open the game
- **WHEN** the same batch is played
- **THEN** goals, shots, expected goals and pass completion per match stay inside their realism bands

### Requirement: A midfielder without the ball does one job in his side's attack
While his side has the ball, a midfielder who is not on it SHALL do one job at a time:
- make the run he has been given, into the penalty area or in behind;
- show for the ball as the short outlet;
- hold the touchline he has been given;
- or hold his place in the shape, behind the ball when he is one of the men kept there.

He SHALL make a run only when one has been given to him. He SHALL NOT be sent forward because a time has passed, because of where on the pitch the ball is, or because his side has just won it. When the ball is lost he SHALL react from where he is, by the same read as any midfielder.

#### Scenario: The man kept behind the ball stays
- **WHEN** his side attacks in the final third and a central midfielder is one of the men kept behind the ball
- **THEN** he holds his place behind the ball and does not run into the penalty area

#### Scenario: The runner runs
- **WHEN** a midfielder is given a place in the penalty area to attack
- **THEN** he makes that run, and no other midfielder makes it with him

#### Scenario: Nobody goes on a clock
- **WHEN** a midfielder with no job in the attack has been holding his place for any length of time
- **THEN** he is still holding his place, and has not been sent forward for the time that has passed

#### Scenario: Winning the ball does not send everybody
- **WHEN** a side wins the ball in midfield
- **THEN** only the midfielders the attack names for a run go forward, and the others hold their places or show for the ball

#### Scenario: The men behind the ball are behind it
- **WHEN** a 100-match batch is played
- **THEN** a midfielder the attack keeps behind the ball is behind it on at least 90% of the ticks he is kept there

### Requirement: A midfielder's time on the ball is counted from when he gets it
Any rule that times how long a midfielder has had the ball, to make him release it under pressure, to end a take-on or to stop him holding it for ever, SHALL count from the moment he got it. It SHALL NOT count how long he had been in some state before that, and it SHALL be counted on the clock it is compared with.

#### Scenario: A pressed midfielder gets the time his composure buys
- **WHEN** a midfielder who has been running without the ball for ten seconds receives it with an opponent 2 m away
- **THEN** he is not forced to release it before the time his composure allows has passed since he received it

#### Scenario: A take-on lasts as long as it says
- **WHEN** a midfielder takes a man on with a time allowance of 1.5 s
- **THEN** the take-on is ended by that allowance 1.5 s after it began, not 3 s after

### Requirement: A forward's readiness follows the play
A forward holding his place in his side's shape SHALL be in one of two phases:
- **Relaxed:** play is far from his place.
- **Alert:** the ball is near his place, or the opposition has it within reach of his place, or it is loose near it.

The phase SHALL follow possession and the ball's distance from his place, whichever side has the ball. Entering and leaving a phase SHALL use different thresholds, so that the phase does not flicker. A tired forward SHALL be relaxed at a shorter distance than a fresh one. The phase SHALL change how he holds himself and how fast he moves. It SHALL NOT change the place he holds.

No state SHALL stop a forward where he stands while his side's shape moves away from him.

#### Scenario: Relaxed while play is far away
- **WHEN** the opposition has the ball more than 50 m from a forward's place, and he is not the man to close it
- **THEN** a forward on his place is relaxed

#### Scenario: Relaxed with his own side on the ball
- **WHEN** his side has settled possession with the ball more than 35 m from his place, and the attack has given him no job
- **THEN** a forward on his place is relaxed, and he is not sent anywhere because either side has the ball

#### Scenario: Alert when the ball comes near
- **WHEN** the opposition has the ball within 35 m of a forward's place
- **THEN** he is alert, whichever state he was in

#### Scenario: The same place in both phases
- **WHEN** a forward changes from relaxed to alert with nothing else changed
- **THEN** the point he steers at is the same before and after

#### Scenario: The phase does not flicker
- **WHEN** a 100-match batch is played
- **THEN** a forward changes between relaxed and alert fewer than 6 times a minute, and fewer than 10% of those changes reverse within 300 ms

#### Scenario: A forward has somewhere to stand
- **WHEN** a 100-match batch is played
- **THEN** a forward spends at least 25% of the match holding his place, relaxed or alert, against 6% before this change

### Requirement: Every forward state reads a situation the same way
A forward without the ball SHALL make the same decision about the same situation whichever state he is in. In particular:
- he plays a ball that is his;
- he goes up for a header he has won;
- he chases a loose ball only when his side's election gives it to him;
- he engages a carrier when he is the man for him, and only then;
- with his own side on the ball, he does the job the attack's assignment gave him;
- he gets back to his place when he is out of it, or beyond the opposition's last line, and has no job that keeps him there.

No state SHALL hand a forward to an idle state, or to the state he carries the ball in, for the decision to be taken again there. A forward SHALL NOT leave an off-ball state because a fixed time has passed.

#### Scenario: A carrier near a forward who is not the man for him
- **WHEN** an opponent carries the ball 6 m from a forward who is not the man for him
- **THEN** the forward does not close him, whether he was walking, holding his place, getting back, holding a touchline or showing for the ball

#### Scenario: A carrier near the man for him
- **WHEN** a forward is the man for a carrier
- **THEN** he engages him from whichever off-ball state he was in

#### Scenario: The ball is lost during a run
- **WHEN** a side loses the ball while one forward is making a run in behind and another is attacking a place in the box
- **THEN** on the next AI tick each of them is doing what the read gives him against the opposition's possession, and neither carries on with his attacking job

#### Scenario: The ball arrives
- **WHEN** a forward gets the ball in any off-ball state
- **THEN** on that AI tick he is in the state that decides what to do with it, and no other state takes that decision for him

#### Scenario: One answer from every state
- **WHEN** the same situation is put to a forward in each state he can hold without the ball
- **THEN** each state's decision is the same one

#### Scenario: A chase the election will refuse is not started
- **WHEN** a 30-match batch is played
- **THEN** fewer than 20% of the loose-ball chases a forward's own decision starts are reversed within ten AI ticks, against about 70% before this change

#### Scenario: The state that carries the ball holds the man on the ball
- **WHEN** a 100-match batch is played
- **THEN** a forward has the ball for at least 95% of the time he spends in the state that decides what to do with it, against 5% before this change

#### Scenario: Less churn
- **WHEN** a 30-match batch is played
- **THEN** no two of a forward's off-ball states exchange more than half the round trips of the largest pair before this change (1,093 a match)
- **AND** no door of a forward without the ball is proposed more than 1,000 times a match and refused on more than half of them, against 4,898 proposals and 4,865 refusals for the largest before it
- **AND** the batch reports a forward's state changes a minute against the ceiling of 22, against 28 before this change

### Requirement: A forward engages a carrier by closing, containing and challenging
A forward who engages a carrier SHALL do it in three phases, each a state of its own:
- **Closing:** he runs to the carrier.
- **Containing:** inside challenge range and goal-side of the carrier, he stays on him and decides when to challenge.
- **Challenging:** the challenge is made, and is decided once.

A forward SHALL be in the challenging state only to make a challenge. While containing he SHALL stand off the carrier on the carrier's goal side; he SHALL NOT run at the man. A forward who is containing SHALL go back to closing when the carrier gets goal-side of him or leaves him behind, and SHALL move no faster than a hard run, so a carrier at full stride can gain on him.

How a forward arrives SHALL count. A challenge made while he is still closing faster than the carrier is moving SHALL be less likely to win the ball, and more likely to be a foul, than the same challenge made after he has set himself.

A forward the carrier has gone past SHALL be able to make the stretched challenge from behind that a beaten defender makes, with its longer recovery.

A forward SHALL keep closing for as long as he is the man for the carrier. He SHALL NOT give the carrier up because of which half of the pitch the ball is in.

A ball that comes loose near the duel SHALL be won through his side's loose-ball election, as from any other state. A forward who wins the ball while closing SHALL play it from the state that decides what to do with the ball.

#### Scenario: A forward contains before he challenges
- **WHEN** a forward reaches challenge range goal-side of a carrier and does not challenge at once
- **THEN** he is in the containing state, on the carrier's goal side, until he challenges, the carrier releases the ball or the carrier gets away from him

#### Scenario: The tackle state counts tackles
- **WHEN** a 100-match batch is played
- **THEN** the number of times forwards enter the challenging state equals the number of challenges they make, against 90 entries for 3.5 challenges a match before this change

#### Scenario: A carrier who gets goal-side is chased
- **WHEN** a carrier goes past the forward containing him
- **THEN** the forward is closing again, and the only challenge open to him is the stretched one from behind

#### Scenario: Diving in
- **WHEN** two equal forwards challenge the same carrier, one arriving at a sprint and one who has been set for half a second
- **THEN** the forward who was set wins the ball more often and fouls less often

#### Scenario: The press does not argue with itself
- **WHEN** a forward who is the man for a carrier follows him into the forward's own half
- **THEN** he keeps closing him, and is not sent back to his place and out again

#### Scenario: A loose ball beside the duel
- **WHEN** the ball comes loose within reach of a forward who was containing its carrier
- **THEN** he goes for it only if his side's election gives it to him, and no rule of his challenging state takes it for him

#### Scenario: Pressing from the front still wins the ball
- **WHEN** a 300-match batch is played
- **THEN** successful pressures per match are no fewer than the 13.0 before this change, and fouls per match stay inside their realism band

### Requirement: A forward out of his place gets back to it, onside
A forward who is out of his place in the shape and has no job that keeps him out SHALL get back to it, in one state, whichever side has the ball. A forward who is beyond the opposition's last line while his side does not have the ball SHALL come back inside it. He SHALL engage the carrier on the way if he becomes the man for him, and SHALL take up a job in the attack on the way if his side wins the ball and gives him one.

#### Scenario: Beyond the line when the ball is lost
- **WHEN** a side loses the ball with a forward standing beyond the opposition's last line
- **THEN** he comes back inside the line, whichever state he was in

#### Scenario: Getting back is not a press
- **WHEN** a forward getting back to his place passes within 15 m of a carrier he is not the man for
- **THEN** he carries on to his place

#### Scenario: The ball is won on the way
- **WHEN** a forward's side wins the ball while he is getting back to his place, and the attack names him for a run
- **THEN** he makes the run from where he is

### Requirement: A header a forward has won is played by him, once
A forward who has won a high ball SHALL go up for it in his heading state and play it himself. That state SHALL carry the header out and SHALL NOT decide the contest. A forward SHALL NOT head a ball that no contest has awarded him.

What he heads SHALL be recorded as what it is: an attempt at goal as a shot; a header away from his own goal inside his own third as a clearance; any other header as neither.

#### Scenario: The winner goes up
- **WHEN** a forward wins the contest for a cross
- **THEN** he is in his heading state, off the ground, when the ball is headed

#### Scenario: No second roll
- **WHEN** a forward who has been awarded a high ball reaches it
- **THEN** he heads it, and nothing rolls again for whether he wins it

#### Scenario: A ball nobody awarded him
- **WHEN** a high ball that no contest has awarded to anybody passes within reach of a forward
- **THEN** his heading state does not roll for it against an opponent

#### Scenario: A knock-down is not a clearance
- **WHEN** a forward glances a won header away from goal inside the opposition's penalty area
- **THEN** neither a shot nor a clearance is recorded for him

#### Scenario: A ball awarded to another man is left
- **WHEN** a high ball has been awarded to one player and a forward, in any state, is within heading reach of it
- **THEN** the forward does not head it

#### Scenario: The heading state heads
- **WHEN** a 100-match batch is played
- **THEN** at least 90% of the headers forwards play are played by a forward in his heading state, off the ground
- **AND** fewer than 20% of a forward's entries into that state end without a header, against 86% before this change

### Requirement: A forward moves at the pace the play asks for
A forward's pace SHALL follow what he is doing and how soon the play can reach him:
- holding his place with time in hand, he walks;
- as the ball comes within reach of his place he jogs, then runs;
- with his own side on the ball and no job given to him, he keeps pace with his place as the shape moves, and no more;
- waiting on the shoulder of the last defender, he moves at the pace of the line;
- waiting for a delivery, he moves no faster than a jog;
- when the ball is played to him in behind, and when a delivery is struck into the box, he runs at his full effort;
- while the ball is dead he walks, unless he cannot be in his place before the restart is taken.

No rule SHALL slow a forward without the ball, or restore his speed, because of how long he has been in a state. The rule that recalls a forward to his place SHALL run at the same pace as his own movement would, for the same situation.

#### Scenario: Play far away
- **WHEN** a forward is on his place and the ball is 50 m away with the opposition on it
- **THEN** he moves no faster than 2 m/s

#### Scenario: On the shoulder
- **WHEN** the runner in behind is level with the last defender, the line is moving at a walk, and no ball has been played to him
- **THEN** he moves at a walk

#### Scenario: The ball is played in behind
- **WHEN** a pass is struck for the runner in behind
- **THEN** he runs at his full effort from that moment

#### Scenario: Attacking a cross
- **WHEN** a delivery is struck toward the place in the box a forward has been given
- **THEN** he may reach his full effort on the way to it, against 52% of his top speed before this change

#### Scenario: Time in a state is not tiredness
- **WHEN** two equally fresh forwards without the ball run to the same point, one who entered his state four seconds ago and one who entered it this tick
- **THEN** both run at the same speed

#### Scenario: A forward's running is reported against the real game
- **WHEN** a 300-match batch is played
- **THEN** the batch reports a forward's distance per 90 minutes and his share of live play at or below 2 m/s against the real 9.5 to 11.5 km and 50% to 75%
- **AND** it reports the mean speed of a runner in behind who has not been played the ball

#### Scenario: Walking forwards do not close the game
- **WHEN** the same batch is played
- **THEN** goals, shots, expected goals and pass completion per match stay inside their realism bands

### Requirement: A forward without the ball does one job in his side's attack
While his side has the ball, a forward who is not on it SHALL do one job at a time:
- make the run in behind he has been given;
- attack the place in the penalty area he has been given;
- show for the ball as the short outlet;
- hold the touchline he has been given;
- or hold his place on the last line.

He SHALL take up a job only when it has been given to him. He SHALL NOT be sent on a run because a time has passed, because of a roll against one of his attributes, because of how far the ball is from him, or because his side has just won it. He SHALL keep a job until it ends: the ball is his, his side has lost it, or the job is no longer his. He SHALL NOT give a job up on a roll.

#### Scenario: Nobody goes on a roll
- **WHEN** a forward with no job in the attack has been holding his place for any length of time
- **THEN** he is still holding his place, whatever his pace or his movement off the ball

#### Scenario: Winning the ball does not send everybody
- **WHEN** a side with two strikers wins the ball in midfield
- **THEN** at most one of them makes a run in behind, and the other holds his place, shows for the ball or attacks a place in the box

#### Scenario: A job is kept
- **WHEN** a forward has been showing for the ball for two seconds, his side still has it and he is still the short outlet
- **THEN** he is still showing for it

#### Scenario: He waits, then attacks the box
- **WHEN** a forward has been given a place in the penalty area and the delivery has not yet been struck
- **THEN** he is waiting off that place, on the blind side of his marker
- **AND** when the delivery is struck he attacks the place

#### Scenario: Two strikers are two players
- **WHEN** a 300-match batch is played in a formation with two strikers
- **THEN** their two position maps are less alike than the 0.66 before this change

### Requirement: A run in behind is made by one man, when the pass is on
A run in behind SHALL be made by the player his side's attack has named for it, and by nobody else. He SHALL wait on the shoulder of the opposition's last defender, inside the line the referee would use, by a margin that follows his timing: a forward who times runs well waits just inside it, and one who does not strays past it. He SHALL go beyond the line when the ball is played to him, and not because some other ball is moving toward him.

The run SHALL end when the ball is his, when his side loses the ball, or when the run is no longer his. It SHALL NOT end because an opponent is standing near him.

The line he holds and the line that decides whether he may start SHALL be the same one.

#### Scenario: He waits inside the line
- **WHEN** the runner in behind is waiting for a pass and no ball has been played to him
- **THEN** he is no further beyond the opposition's last line than his timing allows, and a forward with the best timing is inside it

#### Scenario: A pass to somebody else does not release him
- **WHEN** a pass is struck to a team-mate and happens to travel toward the runner
- **THEN** he stays inside the line

#### Scenario: His marker does not end the run
- **WHEN** the runner in behind has the last defender within 3 m of him
- **THEN** he is still the runner, and does not give the run up for that

#### Scenario: The run ends with the possession
- **WHEN** his side loses the ball while he is waiting on the shoulder
- **THEN** on the next AI tick he is no longer making the run

#### Scenario: Runs are found, and flagged, at a real rate
- **WHEN** a 300-match batch is played
- **THEN** offsides per match stay inside their realism band, and the batch reports runs in behind made, the share played the ball, and the share flagged

### Requirement: A forward's time on the ball is counted from when he gets it
Any rule that times how long a forward has had the ball, to make him release it, to let him strike it early or to stop him holding it for ever, SHALL count from the moment he got it. It SHALL NOT count how long he had been in some state before that, and it SHALL be counted on the clock it is compared with.

#### Scenario: A forward who has waited gets his time
- **WHEN** a forward who has been without the ball for ten seconds receives it inside the penalty area with no shot on
- **THEN** he is not made to give it up before the time a forward is allowed there has passed since he received it

#### Scenario: A first-time strike is first time
- **WHEN** a forward has had the ball for a second
- **THEN** a rule that applies only to the first moment after receiving it no longer applies to him, however recently he changed state

### Requirement: A wide carrier takes the byline, then the box
A carrier in a wide area of the attacking third who is being shown down the line, or whose run down the outside is open, SHALL drive toward the byline. This holds whatever his role and position in the formation.

On reaching the byline zone, if the grass along the goal line toward the near post is open, he SHALL carry the ball on along it into the width of the penalty area. He SHALL NOT stop at the corner of the area. The grass counts as open when no opponent is on that line within a few strides of him.

He SHALL stop carrying and play the ball when any of the following happens:
- an opponent closes that line;
- a team-mate's run makes the ball on;
- he reaches the edge of the six-yard box.

A carrier whose inside lane is open SHALL remain free to cut inside instead. How open that lane must be SHALL stay his own preference.

#### Scenario: A winger with the goal line open goes into the box
- **WHEN** a winger reaches the byline at the corner of the penalty area, no defender stands between him and the near post along the goal line, and team-mates are arriving in the box
- **THEN** he carries the ball along the goal line into the width of the penalty area before he delivers it

#### Scenario: A defender on the goal line stops the carry
- **WHEN** a defender stands on the goal line between the carrier and the near post, a stride or two from him
- **THEN** the carrier does not run into him, and plays the ball from where he is or turns back

#### Scenario: The full-back who overlapped does the same
- **WHEN** a full-back who has overlapped receives the ball wide in the attacking third with the run down the outside open
- **THEN** he drives the byline and, with the goal line open, carries on into the box exactly as a winger would

#### Scenario: An open inside lane is still his choice
- **WHEN** a wide carrier who prefers to cut inside has an open lane inside him
- **THEN** he may cut inside instead of driving the byline

### Requirement: A ball from the byline inside the box is pulled back or driven across
A carrier delivering from inside the width of the penalty area, near the goal line, SHALL choose between three balls:
- a ball pulled back to a team-mate arriving at the edge of the area or the penalty spot;
- a ball driven low across the six-yard box in front of the keeper;
- a ball chipped to the far post.

He SHALL choose by where his team-mates are arriving and where the defenders and the keeper stand. One team-mate arriving in the box SHALL be enough for him to deliver from there.

While a team-mate carries the ball along the byline into the box, the side's box runners SHALL attack the near post and arrive at the edge of the area.

#### Scenario: A free runner at the edge gets the pull-back
- **WHEN** the carrier is inside the box near the goal line, the defenders have dropped onto the six-yard line, and a team-mate arrives unmarked at the edge of the area
- **THEN** the carrier pulls the ball back to him along the ground

#### Scenario: A crowded edge gets the ball across the six-yard box
- **WHEN** the carrier is inside the box near the goal line, the edge of the area is covered, and a team-mate is attacking the near post
- **THEN** the carrier drives the ball low across the six-yard box in front of the keeper

#### Scenario: One runner is enough from the byline
- **WHEN** the carrier is inside the box near the goal line and only one team-mate is arriving in the box
- **THEN** he may still deliver to him

#### Scenario: The runners move for the byline ball
- **WHEN** a team-mate carries the ball along the goal line into the box
- **THEN** the side's box runners re-aim, one at the near post and one at the edge of the area, rather than holding the slots they took for a cross from deep

### Requirement: A player pinned at his own touchline with no safe out puts the ball out
A player in possession in his own half SHALL play the ball over the touchline, out of play, when both of these hold:
- an opponent's pressure pins him against his own touchline;
- he has no safe pass and no room to carry out.

He SHALL do this rather than play into the press or back across his own goal. The rule SHALL be the same for every role.

The decision SHALL weigh the same four readings as a defender's clearance call in his own penalty area:
- how close the pressure is;
- how near his own goal he is;
- whether a safe out exists;
- his own ability to play out under pressure.

A composed player who plays out from the back SHALL ride pressure that makes a limited one put the ball out. When he puts it out, he SHALL play it up the line, away from his own goal.

#### Scenario: Pinned with no out, he puts it out
- **WHEN** a full-back with the ball is pressed by an opponent within a stride, against his own touchline in his own third, with no team-mate he can safely reach and the line ahead closed
- **THEN** he plays the ball out of play over the touchline, up the line, and the opponents get a throw-in

#### Scenario: A safe pass is played instead
- **WHEN** the same full-back has a safe pass to a team-mate
- **THEN** he plays the pass and does not put the ball out

#### Scenario: A wide midfielder pinned in his own half does the same
- **WHEN** a wide midfielder who has tracked back is pinned with the ball against his own touchline in his own third, with no safe pass and no room to carry
- **THEN** he puts the ball out up the line, exactly as a full-back would

#### Scenario: The limited player puts it out more readily
- **WHEN** a composed player who plays out from the back and a limited one are each pinned at the touchline under the same pressure with the same outs
- **THEN** the limited player puts the ball out more often than the composed one
