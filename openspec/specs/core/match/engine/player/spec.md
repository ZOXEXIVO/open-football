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
A defending player near an opponent in possession SHALL be able to attempt a tackle, with the outcome influenced by both players' relevant attributes and the contact severity.

#### Scenario: Tackle outcome depends on both players' attributes
- **WHEN** a defender attempts a tackle against an opponent in possession
- **THEN** the outcome of the duel is influenced by both the tackler's and the ball-carrier's relevant attributes and by the contact severity, rather than being decided by one side alone

### Requirement: Goalkeeper shot-stopping
The goalkeeper SHALL attempt to stop shots on target within their reach, choosing among save actions (catching, punching, diving) appropriate to the shot's trajectory, speed and proximity, with save success influenced by the keeper's positioning, reach and relevant skills.

#### Scenario: Close-range shot inside reach is saveable
- **WHEN** a shot on target falls within the goalkeeper's positional reach envelope
- **THEN** the keeper has a save opportunity via the appropriate save action rather than automatically conceding

### Requirement: Goalkeeper distribution and positioning
The goalkeeper SHALL distribute the ball after gaining possession (via throw, kick, or pass) according to the tactical situation and available options, and SHALL adjust their position between the goal line and the edge of the area based on the location of play.

#### Scenario: Keeper comes off the line to sweep
- **WHEN** the ball is played into space behind the defensive line with the goalkeeper closest to reach it
- **THEN** the goalkeeper can advance from the goal to intercept or clear it rather than remaining fixed on the line

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
A player's effective on-pitch capability (maximum speed and the outcome of skill-dependent actions) SHALL be reduced as their condition falls, so a tired player performs below a fresh player of identical underlying ability.

#### Scenario: Tired player's top speed is reduced
- **WHEN** a player's condition has dropped substantially below full
- **THEN** their maximum attainable speed for the remainder of that spell is lower than at full condition

### Requirement: In-match injury state
A player SHALL be able to sustain an in-match injury that removes them from active play — they stop moving, recover no condition, and are excluded from ball-chase and loose-ball contests — until a periodic medical check resolves their status.

#### Scenario: Injured player is excluded from active play
- **WHEN** a player is marked injured during the match
- **THEN** that player stops moving, recovers no condition, and is excluded from ball-chase and loose-ball contests until the next medical check resolves their status

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
