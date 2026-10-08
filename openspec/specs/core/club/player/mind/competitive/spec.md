# core/club/player/mind/competitive Specification

## Purpose
The competitive mind holds a player's self-belief and his assurance, the standard of football he is used to. It
covers what moves each of them, how they are seeded when a world is built, and the state of mind he brings to
kickoff in place of his career statistics.

## Requirements

### Requirement: Assurance is the standard of football a player is used to
Every player SHALL carry an assurance: the standard of football he is used to. It SHALL be measured on the same scale
as the standard of football a match reads from both sides at kickoff.

Assurance SHALL change only through the football he plays and the time he goes without it:
- **Above his assurance:** a match he plays at a standard above it SHALL pull his assurance toward that standard, in
  proportion to the minutes he plays. A full match SHALL move it more than a substitute appearance, and a friendly
  SHALL move it less than a competitive match of the same standard and minutes.
- **Pace:** composure and adaptability SHALL set how fast it moves. A composed, adaptable player SHALL close more of
  the gap over the same matches than a nervous, poorly adapting one.
- **Below his assurance:** a match at a lower standard SHALL lower his assurance, more slowly than the same minutes
  above it would raise it.
- **Absence:** without matches his assurance SHALL fade, by a bounded amount, so that a player returning from a long
  absence is never treated as a debutant.
- **Outcome-blind:** the result of the match, his rating and his errors SHALL NOT affect how his assurance moves.

#### Scenario: A reserve keeper promoted to the first team settles over a dozen matches
- **WHEN** a keeper of median composure and adaptability, assured at his club's reserve standard, plays 12 full
  competitive matches at the first team's higher standard
- **THEN** his assurance has closed at least half and at most nine-tenths of the gap between the two standards

#### Scenario: How the match went does not change assurance
- **WHEN** two otherwise identical players play the same minutes at the same standard, and one has the best match of
  his career while the other makes two errors leading to goals
- **THEN** their assurance moves by the same amount

#### Scenario: A composed player settles faster
- **WHEN** two players step up to the same standard and play the same matches, one composed and adaptable and the
  other nervous and slow to adapt
- **THEN** the composed player's assurance ends closer to the new standard

#### Scenario: A loan down a level lowers assurance slowly
- **WHEN** a player assured at a top-flight standard plays a season of full matches one division below
- **THEN** his assurance falls by less than it would have risen over the same minutes one division above

#### Scenario: A cameo counts for less than a full match
- **WHEN** one player plays 90 minutes and another 15 minutes at the same standard above both their assurances
- **THEN** the player who played 90 minutes gains more assurance

### Requirement: Assurance steadies self-belief
How far a single event moves a player's self-belief SHALL depend on how assured he is at the standard where it
happened. The same event SHALL move the belief of a player playing at or below his assurance less than the belief of
a player playing above it.

#### Scenario: A veteran shrugs off a bad afternoon a debutant does not
- **WHEN** a veteran keeper assured at the standard of the match and a debutant playing well above his assurance each
  make an error leading to a goal in a match at that standard
- **THEN** the debutant's self-belief falls further than the veteran's

### Requirement: Self-belief is moved by goalkeeping evidence and by real errors
A player's self-belief SHALL be moved by what actually happened to him in a match:
- **Penalty save:** a penalty saved in a competitive match SHALL raise a goalkeeper's self-belief.
- **Match-saving display:** goals prevented beyond what an average keeper would have saved, in a competitive match his
  side did not lose, SHALL raise a goalkeeper's self-belief.
- **Clean sheet:** a clean sheet in a competitive match SHALL raise a goalkeeper's self-belief by a small amount. A run
  of clean sheets SHALL NOT fill the player's memory of significant moments.
- **Costly error:** an error leading to a goal in a competitive match SHALL lower any player's self-belief and be
  remembered as a costly error.
- **Rating alone:** a low match rating with no error behind it SHALL NOT be remembered as a costly error.

#### Scenario: A penalty save lifts a keeper
- **WHEN** a keeper saves a penalty in a competitive match
- **THEN** his self-belief afterwards is higher than before the match

#### Scenario: A heavy defeat without an error is not a costly error
- **WHEN** a keeper concedes four goals in a competitive match, makes no error leading to a shot or goal, and receives
  a match rating below 5.5
- **THEN** no costly-error memory is filed for him

#### Scenario: An outfield error is remembered too
- **WHEN** a defender gives the ball away and the opponent scores from it within the error window
- **THEN** a costly-error memory is filed for the defender and his self-belief falls

### Requirement: Assurance is seeded from the real career when a world is built
When a world is constructed, each player's assurance SHALL be seeded once. The seed SHALL be the assurance the growth
and erosion rules would have produced over his recorded career seasons, played in order:
- each season is read at the standard of football of the club he played for, with the appearances he made there;
- a club's standard SHALL be its squad's standard when the club exists in the world, otherwise its league's standard,
  otherwise one derived from the club's recorded reputation.

A player with no recorded career, whether present when the world is built or created later (an academy intake, a
newly generated player), SHALL start from his squad role: a regular starter at his team's standard, a squad player
below it, and a youth player at his age group's standard.

Self-belief SHALL start neutral.

#### Scenario: A veteran starts assured
- **WHEN** a world is built containing a 34-year-old keeper with ten recorded top-flight seasons of 30 or more
  appearances
- **THEN** his starting assurance sits at or near his top-flight club's standard

#### Scenario: A third-choice keeper with no senior football starts unassured
- **WHEN** a world is built containing a 19-year-old keeper at a top-flight club whose recorded career has no senior
  appearances
- **THEN** his starting assurance sits well below the first team's standard

#### Scenario: A returning loanee starts at the loan level
- **WHEN** a world is built containing a keeper whose last two recorded seasons are full seasons on loan at a
  second-tier club
- **THEN** his starting assurance sits nearer the second-tier club's standard than his parent club's

#### Scenario: An academy intake starts at his age group
- **WHEN** a keeper joins a club's academy in an annual intake during play
- **THEN** his starting assurance is his age group's standard, not the first team's

### Requirement: A player brings his state of mind to kickoff, never his statistics
At kickoff the match SHALL receive, for each player:
- his assurance;
- his self-belief;
- his morale;
- the temperament attributes that answer an occasion: how he copes with pressure, and how he rises to important
  matches.

It SHALL also receive his big-match record. The match SHALL NOT receive his career appearances, minutes, caps or
history.

#### Scenario: Career length is invisible to the match
- **WHEN** two players with identical skills, assurance, self-belief, morale, temperament and big-match record, but
  different career appearance totals, start the same match
- **THEN** their kickoff state of mind is identical
