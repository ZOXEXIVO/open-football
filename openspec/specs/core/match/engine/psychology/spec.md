# core/match/engine/psychology Specification

## Purpose
Tracks each player's in-match psychological state — composure, confidence, momentum — and feeds it back into that player's on-pitch execution.

## Requirements

### Requirement: In-match player psychology
A player's in-match psychological state (such as composure and confidence) SHALL be able to shift based on match events, and SHALL feed back into that player's effective execution of skill-dependent actions (e.g. passing accuracy) as a bounded modifier rather than an unbounded swing.

#### Scenario: Low composure reduces pass accuracy
- **WHEN** a player's psychological state reflects reduced composure and first-touch reliability
- **THEN** that player's evaluated pass success probability is nudged downward relative to a neutral psychological state, within a small bounded range

### Requirement: Kickoff confidence and nerves come from the player's state of mind and the occasion
Every player's in-match psychology SHALL be seeded at kickoff instead of starting neutral:
- **Confidence** SHALL rise with his self-belief and his morale.
- **Nerves** SHALL rise with the step up and with the occasion:
  - the step up is the amount by which the match's standard of football exceeds his assurance;
  - the occasion is the fixture's importance, expected crowd, rivalry and whether it is a knockout tie.
- **What damps nerves:** his pressure-handling temperament, his composure, his big-match record and his self-belief.
  A player who rises to important matches SHALL feel less of the occasion.
- **At home:** a player playing at or below his assurance in an ordinary fixture SHALL start with negligible nerves.

#### Scenario: A reserve keeper's first senior start
- **WHEN** a keeper assured at reserve standard starts a first-team league match at a standard well above it
- **THEN** his kickoff nerves are higher than those of the same keeper starting a reserve match

#### Scenario: A veteran at his usual standard is calm
- **WHEN** a keeper assured at the standard of the match starts an ordinary league fixture
- **THEN** his kickoff nerves are negligible

#### Scenario: The occasion raises nerves on its own
- **WHEN** the same keeper, assured at the standard of the match, starts a cup final instead of an ordinary league
  fixture at the same standard
- **THEN** his kickoff nerves are higher in the cup final

#### Scenario: Temperament answers the occasion
- **WHEN** two players with identical assurance and belief start the same high-stakes fixture, one strong and one
  weak at handling pressure
- **THEN** the player strong at handling pressure starts with lower nerves

### Requirement: Psychology modifiers are continuous and bounded
The modifiers in-match psychology applies to execution and to the decision to attempt an action SHALL be continuous
in confidence and nerves, with no thresholds at which an effect switches on. A neutral state SHALL produce exactly
neutral modifiers, and every modifier SHALL stay within a bounded range.

#### Scenario: A small change in confidence makes a small change
- **WHEN** a player's confidence moves by a small amount anywhere in its range
- **THEN** each psychology modifier moves by a correspondingly small amount, never by a step

#### Scenario: A neutral head is neutral
- **WHEN** a player's confidence and nerves are both neutral
- **THEN** every psychology modifier leaves his execution and his decisions unchanged

### Requirement: Errors and big saves move in-match confidence
During a match:
- an error leading to a shot SHALL lower the player's confidence;
- an error leading to a goal SHALL lower it further;
- a save that keeps out a high-quality chance SHALL raise the keeper's confidence;
- a goal conceded SHALL lower the keeper's confidence by less than an error of his own leading to a goal.

The changed confidence SHALL stay in force for the rest of his time on the pitch, so his later actions read it.

#### Scenario: An early error colours the rest of a keeper's match
- **WHEN** a keeper makes an error leading to a goal in the tenth minute
- **THEN** his confidence for the rest of the match is lower than that of the same keeper in the same match without
  the error

#### Scenario: A big save lifts him
- **WHEN** a keeper saves a shot of high expected-goal value
- **THEN** his confidence afterwards is higher than before the save

### Requirement: Every player carries the same psychology, with bounded outfield effects
Kickoff seeding and in-match updates SHALL apply to every player on the pitch, not only to goalkeepers. A player's
state of mind SHALL reach his play through two channels, and both SHALL work the same way for every player:
- **Reliability:** nerves and low confidence SHALL raise how often his actions fail, through one factor shared by
  every player.
  - Each kind of action SHALL feel that factor in proportion to how much of its failure is a lapse rather than a
    matter of skill. A fluffed touch or a miscued clearance SHALL feel it fully; a missed shot or a mistimed
    challenge SHALL feel only a share of it.
  - A failure that is already common SHALL grow toward certainty without reaching it.
  - A goalkeeper and an outfield player with the same confidence and nerves SHALL see the same kind of action's
    failure rate rise by the same factor over a settled state.
- **Initiative:** confidence SHALL raise his willingness to attempt actions, and nerves SHALL lower it.

How steady a player is SHALL scale how often his actions fail, separately from his state of mind. Steadiness comes
from his concentration, composure, decisions and consistency.

A neutral head SHALL leave every failure rate where his steadiness and the situation put it.

These effects SHALL be bounded so that each of the following stays within its calibration band:
- population pass accuracy;
- fouls and penalties per match;
- goals per match.

The miscontrol rate SHALL NOT rise above its baseline beyond run-to-run noise.

#### Scenario: A nervous outfielder miscontrols more
- **WHEN** two outfield players of identical attributes play the same matches, one starting nervous and the other
  calm
- **THEN** the nervous player's miscontrol rate over those matches is higher

#### Scenario: Keeper and outfielder share one reliability factor
- **WHEN** a goalkeeper and an outfield player carry the same confidence and nerves
- **THEN** the factor by which each one's failure rate for the same kind of action rises over his settled rate is
  the same

#### Scenario: A skill-heavy action feels less of his head
- **WHEN** a nervous player shoots and receives passes whose settled failure rates are the same
- **THEN** his rate of fluffed touches rises by more than his rate of missed shots

#### Scenario: A steady player fails less with the same state of mind
- **WHEN** two outfield players with the same state of mind make the same kind of action in the same situation, one
  with high concentration, composure, decisions and consistency and the other with low
- **THEN** the steadier player's action fails less often

#### Scenario: Population calibration holds
- **WHEN** the seeded realism batch is run with every player's reliability lane in force
- **THEN** pass accuracy, fouls, penalties and goals per match each stay within their calibration bands
- **AND** miscontrols per match do not rise above the baseline beyond run-to-run noise

### Requirement: Nerves settle as a player plays himself in
The nerves a player brings to kickoff SHALL ease with his time on the pitch:
- **Pace:** a composed player SHALL settle faster than an anxious one.
- **Floor:** the easing SHALL stop at a residual share of his kickoff nerves for the rest of that match, so a big
  step up is still felt at the final whistle.
- **Substitutes:** a substitute SHALL settle from the moment he enters.
- **Other sources:** nerves from other causes, such as a booking or the closing stage of a close match, SHALL NOT be
  eased by this settling.

#### Scenario: A debutant settles but does not become a veteran
- **WHEN** a player starts a match well above his assurance
- **THEN** his nerves an hour in are lower than at kickoff
- **AND** they remain higher than those of a player assured at that standard

#### Scenario: A composed player settles faster
- **WHEN** two players start the same match with the same kickoff nerves, one composed and the other anxious
- **THEN** half an hour in, the composed player's nerves are lower

#### Scenario: A substitute settles from his entry
- **WHEN** a nervous substitute enters in the 70th minute
- **THEN** his nerves on entry are his full kickoff nerves and ease from that point on

### Requirement: A close match presses harder as it nears its end
In the closing stage of a match whose score is level or separated by one goal, every player on the pitch SHALL carry
added nerves that grow toward the final whistle:
- **Temperament:** a player strong at handling pressure SHALL feel less of it.
- **Leadership:** a commanding leader of his own side on the pitch SHALL damp it.
- **Stakes:** it SHALL weigh more in a fixture with more at stake.
- **Margin:** it SHALL NOT apply while the margin is two goals or more.
- **Duration:** it SHALL fall away as soon as the margin grows.

The added nerves SHALL act through the same reliability and initiative channels as any other nerves.

#### Scenario: The last minutes of a level match are tenser than the first
- **WHEN** the same player is on the pitch at 1–1 in the 10th minute and at 1–1 in the 85th minute
- **THEN** his nerves are higher in the 85th minute

#### Scenario: Temperament answers the moment
- **WHEN** two players with identical kickoff states are on the pitch in the 85th minute of a level match, one strong
  and one weak at handling pressure
- **THEN** the player weak at handling pressure carries more nerves

#### Scenario: A leader on the pitch calms his side
- **WHEN** the same side is level in the 85th minute, once with a commanding leader on the pitch and once without one
- **THEN** its players carry fewer added nerves when the leader is on the pitch

#### Scenario: A comfortable lead is calm
- **WHEN** a side leads by three goals in the 85th minute
- **THEN** no player on the pitch carries nerves from the closing stage
