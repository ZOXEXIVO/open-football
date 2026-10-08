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
Kickoff seeding and in-match updates SHALL apply to every player on the pitch, not only to goalkeepers:
- nerves SHALL raise an outfield player's miscontrol, rushed-clearance and foul risks;
- confidence SHALL raise his willingness to attempt actions.

These outfield effects SHALL be bounded so that population pass accuracy, miscontrol rate and goals per match stay
within their calibration bands.

#### Scenario: A nervous outfielder miscontrols more
- **WHEN** two outfield players of identical attributes play the same matches, one starting nervous and the other
  calm
- **THEN** the nervous player's miscontrol rate over those matches is higher

#### Scenario: Population calibration holds
- **WHEN** the seeded realism batch is run with kickoff seeding in force
- **THEN** pass accuracy, miscontrol rate and goals per match each stay within their calibration bands
