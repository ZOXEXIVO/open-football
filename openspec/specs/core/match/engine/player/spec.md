# core/match/engine/player Specification

## Purpose
Owns individual player behavior on the pitch: passing and shooting decisions, tackling, goalkeeper actions, fatigue/condition, injury state, and the per-player stat line those actions accumulate into.

## Requirements

### Requirement: Ball possession and passing decisions
A player in possession SHALL evaluate candidate passes using a combination of distance, passing angle, pressure on the passer, the receiver's positioning and ability, the passer's own ability, and the tactical value of the resulting position, and SHALL weigh the risk that the pass is intercepted before choosing whether and where to pass.

#### Scenario: Pass success probability responds to pressure
- **WHEN** the passer is under close defensive pressure at the moment of the pass
- **THEN** the evaluated success probability for that pass is lower than the same pass attempted with no pressure applied

#### Scenario: Adverse weather reduces pass accuracy
- **WHEN** the match environment carries adverse weather (rain or wind) modifiers
- **THEN** evaluated pass success probability is reduced accordingly, with long passes affected further by wind

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
