# core/match/engine/diagnostics Specification

## Purpose
Owns what the engine measures about its own realism — the per-match census exposed under the `match-logs` feature and the seeded batch acceptance test that fails when headline statistics leave their real-football bands.

## Requirements

### Requirement: Realism census coverage
Under the `match-logs` feature the engine SHALL expose, per match, every headline statistic a real match report carries and that the engine can influence: goals, shots and shots on target, xG, passes and pass accuracy, tackles, interceptions, fouls by source (tackle, holding, professional foul, dissent, time-wasting, handball), cards by source, offsides, corners, throw-ins, free kicks by type (direct, indirect), penalties awarded and scored, own goals, injuries by cause and severity, substitutions by reason, effective playing time (ball-in-play seconds), stoppage time added per period, goals by origin (open play, set piece, penalty, own goal) and goals by strike distance where a rebound finish is attributed to the rebound rather than the original strike.

#### Scenario: Rebound finish is not booked as a long-range goal
- **WHEN** a shot from outside the penalty area is parried and the rebound is finished from inside the six-yard box
- **THEN** the goal-distance census records a goal from the rebound distance, not from the original strike distance

#### Scenario: Set-piece origin is recorded
- **WHEN** a goal is scored within the same possession as a corner, free kick or throw-in without the defending side having gained controlled possession in between
- **THEN** the goal is recorded with a set-piece origin

### Requirement: Seeded batch acceptance bands
The repository SHALL contain a seeded batch test that plays a fixed number of matches between equal-strength sides. It SHALL assert the per-match averages of the following against stated bands:
- goals, shots, shots on target, corners;
- fouls, yellow cards, red cards, offsides, throw-ins, penalties awarded;
- effective playing time and stoppage time per period;
- injuries;
- state changes per player per minute;
- distance covered per outfield player;
- the goalkeepers' save percentage;
- the mean time a goalkeeper spends on the ground after going to ground without the ball;
- a goalkeeper's mean speed while he is on the ground;
- distance covered per defender;
- the share of live play a defender spends at or below a walk;
- state changes per defender per minute.

It SHALL record every band's current value in a stored baseline so a regression is a diff, not a memory. The test MAY be `#[ignore]`d for ordinary runs but MUST be runnable on demand with one command.

#### Scenario: Statistic drifts out of band
- **WHEN** the batch test runs and the average of any band-checked statistic falls outside its stated band
- **THEN** the test fails naming the statistic, the measured value and the band

#### Scenario: Penalties are counted
- **WHEN** a penalty is awarded during a batch match
- **THEN** the batch accumulator's penalty count increases, and the penalties-per-match band is checked rather than always reading zero

#### Scenario: State churn is band-checked
- **WHEN** a change raises outfield state changes per player per minute above 45
- **THEN** the batch report shows that statistic out of band and diffs it against the baseline

#### Scenario: Distance covered is reported against the real band
- **WHEN** the batch runs
- **THEN** it reports distance covered per outfield player per 90 minutes against a real band of 10 to 13 km, so the distance is tracked as a diff from the stored baseline even while it remains out of band

#### Scenario: A keeper on the ground is band-checked
- **WHEN** the batch runs
- **THEN** it checks a goalkeeper's mean time down after going to ground without the ball against a band of 0.6 to 1.2 s, and his mean ground speed while down against a ceiling of 1.5 m/s

#### Scenario: Save percentage is band-checked
- **WHEN** a change moves the goalkeepers' save percentage outside 62% to 76%
- **THEN** the batch fails naming save percentage

#### Scenario: A defender's movement is reported against the real bands
- **WHEN** the batch runs
- **THEN** it reports distance covered per defender per 90 minutes against a real band of 9.5 to 12 km, and the share of live play a defender spends at or below 2 m/s against a real band of 50% to 75%, so both are tracked as a diff from the stored baseline even while they remain out of band

#### Scenario: A defender's churn is band-checked
- **WHEN** a change raises state changes per defender per minute above 20
- **THEN** the batch report shows that statistic out of band and diffs it against the baseline

### Requirement: State machine census
Under the `match-logs` feature the engine SHALL report, per match, how players move through their states, in enough detail that an oscillation cannot hide behind a limit on its rate. The report SHALL include:
- state changes per player per minute, the share that reverse the previous change and the share within one AI tick of entry;
- for every observed transition, its source, its count per match and a histogram of how long the player had been in the state he left;
- the count of cycles through three states;
- the count of transitions a commitment rule held back or the resolution refused, by pair of states;
- the count of times each transition site in the code proposed a change, including proposals that did not take effect;
- for every state, its time occupied, the share of that time with the ball, the share standing still, the mean speed and the effort tier it was served at.

#### Scenario: A rate-limited oscillation is visible
- **WHEN** two states exchange a player as fast as a commitment rule allows
- **THEN** the dwell histogram for that transition shows the concentration at the rule's limit, and the held-back count for that pair is reported

#### Scenario: A cycle of three states is visible
- **WHEN** players cycle through three states with none of them reversing the previous change
- **THEN** the census reports that cycle by name, although no two-state reversal occurred

#### Scenario: A noisy door can be found
- **WHEN** one transition site proposes far more changes than any other
- **THEN** the census names its source location and its proposals per match

#### Scenario: A foul is attributed to the state it was decided in
- **WHEN** a marking defender commits a foul and the same decision moves him to another state
- **THEN** the foul census attributes the foul to the state he was marking in, and not to the state he moved to

### Requirement: Possession reading census
Under the `match-logs` feature the engine SHALL report, per match, how often players' readings of possession change, how often they change while the ball is unowned, how often team-mates disagree, and how often both sides or neither side claim possession.

#### Scenario: A disagreement is counted
- **WHEN** two team-mates read possession differently on the same tick
- **THEN** that tick is counted as a disagreement in the census

### Requirement: Goalkeeper action census
Under the `match-logs` feature the engine SHALL report, per match and per keeper, how goalkeeping actions are decided and how they end. The report SHALL include:
- for every shot that reaches a keeper, which adjudication decided it, and the outcome: held, parried and kept in play, tipped, spilled, deflected or beaten;
- for every high ball a keeper comes for, his chosen action (catch or punch), which adjudication decided it, and the outcome: caught, punched clear, flapped or lost;
- rebounds: every shot struck within 3 s of a keeper's touch, with the keeper's state and height at that strike, and how the shot ended;
- the time a keeper spends on the ground after going to ground, and his speed while he is down;
- the keeper's ground speed at the moment each shot is struck, and the delay it added to his reaction;
- the share of time a keeper spends relaxed, alert and set, and how often he changes between them.

Every per-state table in the keeper diagnostics SHALL be sized from the state set itself. Adding a keeper state SHALL therefore neither crash a census nor drop that state silently from it.

#### Scenario: A second adjudication is visible
- **WHEN** any shot is decided by more than one adjudication in a batch
- **THEN** the census reports the count of such shots, and the expected count is zero

#### Scenario: Rebounds are visible
- **WHEN** a keeper parries a shot and an opponent shoots again within 3 s
- **THEN** the census records the second shot with the keeper's state and height at that strike and with how the shot ended

#### Scenario: A new keeper state is counted
- **WHEN** a keeper enters a state whose id is higher than every id that existed before this change
- **THEN** every keeper census counts that state under its own name, and no census panics or drops it

### Requirement: Defender action census
Under the `match-logs` feature the engine SHALL report, per match, how defenders defend and how each contact they are part of is decided. The report SHALL include:
- for every time the ball is taken off a player who has it, which rule decided it, the taker's role and state, the distance between the two players, and what was credited for it;
- for every spell in which a defender engages a carrier, the time he spent closing and the time he spent containing, his speed when he challenged, the kind of challenge, and how the spell ended: won, missed, a foul, the carrier released the ball, the carrier got away, or the job passed to another man;
- for every header an outfield player plays, which rule decided it, the state he was in when he played it, whether he left the ground, and what it was recorded as;
- for every defender-tick out of possession, the job his side's assignment gave him and the state he was in, with the count of state changes that followed a change of assignment;
- for a fixed list of situations (a carrier within challenge range of the man for him, a loose ball the election gives him, a high ball he has won, his own side in possession, a carrier in his own area), the share of defender-ticks, by state, on which the state acted on the situation;
- distance covered, the share of time in each speed band and the number of sprints, by position in the back line, split by live and dead ball and by state;
- the share of time a defender spends relaxed, alert, on a job, in a duel and recovering, and how often he changes between relaxed and alert;
- for every blocked shot and every ball knocked away by a tackle, the speed it left at, the distance between the defender and the ball at the contact, and how far it travelled before it stopped or was touched.

Tackles SHALL be reported as attempted and won, by role.

Every per-state table in the defender diagnostics SHALL be sized from the state set itself. Adding a defender state SHALL therefore neither crash a census nor drop that state silently from it.

#### Scenario: A second resolver is visible
- **WHEN** the ball is taken off a carrier by a rule other than a challenge he was the target of
- **THEN** the census counts it under that rule's name, with what was credited

#### Scenario: A header decided twice is visible
- **WHEN** any flight is decided by more than one aerial rule in a batch, or a player rolls again for a ball already awarded to him
- **THEN** the census reports the count, and the expected count is zero

#### Scenario: A duel is read phase by phase
- **WHEN** a defender closes a carrier, contains him for a second and wins a tackle
- **THEN** the census records one spell with its closing time, its containing time, the challenge's kind and the outcome won

#### Scenario: A blind door is visible
- **WHEN** a defender in some state does not act on a carrier in his own area although he is one of the two nearest
- **THEN** the door table shows that state acting on that situation on less than all of its ticks

#### Scenario: A defender's distance is split
- **WHEN** the batch runs
- **THEN** the census reports a centre-back's and a full-back's distance per 90 minutes separately, each split into live and dead ball

#### Scenario: A new defender state is counted
- **WHEN** a defender enters a state whose id is higher than every id that existed before this change
- **THEN** every defender census counts that state under its own name, and no census panics or drops it

### Requirement: Midfielder action census
Under the `match-logs` feature the engine SHALL report, per match, how midfielders move, what job each of them has, and how each contact they are part of is decided. The report SHALL include:
- distance covered, the share of time in each speed band and the number of sprints, by position in the midfield, split by which side has the ball, by live and dead ball, and by state;
- for every midfielder-tick out of possession, the job his side's assignment gave him and the state he was in, with the count of state changes that followed a change of assignment;
- for every midfielder-tick in possession, the job the attack's assignment gave him and the state he was in, and whether he was ahead of the ball;
- for every spell in which a midfielder engages a carrier, the time he spent closing and the time he spent containing, his speed when he challenged, the kind of challenge, and how the spell ended: won, missed, a foul, the carrier released the ball, the carrier got away, or the job passed to another man;
- for every ball a midfielder comes away with while engaging a carrier, the rule that gave it to him;
- for every header a midfielder plays and every ball that drops with no named receiver, which rule decided it, the state he was in, whether he left the ground, and what it was recorded as; a high ball he claims uncontested is reported as a reception, apart from his headers;
- for a fixed list of situations (a carrier within challenge range of the man for him, a carrier near a man who is not, a loose ball the election gives him, a high ball he has won, the ball lost by his side, the ball won by it), the share of midfielder-ticks, by state, on which the state acted on the situation;
- the share of time a midfielder spends relaxed, alert, on a job, in a duel, recovering, supporting the attack and on the ball, and how often he changes between relaxed and alert;
- for every loose-ball chase a midfielder's own decision starts, whether his side's election reversed it within ten AI ticks.

Every per-state table in the midfielder diagnostics SHALL be sized from the state set itself. Adding a midfielder state SHALL therefore neither crash a census nor drop that state silently from it.

#### Scenario: A midfielder's distance is split
- **WHEN** the batch runs
- **THEN** the census reports a central midfielder's and a wide midfielder's distance per 90 minutes separately, each split by which side has the ball and by live and dead ball

#### Scenario: A job nobody acts on is visible
- **WHEN** a midfielder is kept behind the ball by the attack's assignment and is ahead of it
- **THEN** the census counts those ticks under that job, by the state he was in

#### Scenario: A ball won without a challenge is visible
- **WHEN** a midfielder comes away with the ball from a carrier he was engaging, by a rule other than a challenge decided against that carrier
- **THEN** the census counts it under that rule's name

#### Scenario: A blind door is visible
- **WHEN** a midfielder in some state does not act on a carrier within challenge range although he is the man for him
- **THEN** the door table shows that state acting on that situation on less than all of its ticks

#### Scenario: A new midfielder state is counted
- **WHEN** a midfielder enters a state whose id is higher than every id that existed before this change
- **THEN** every midfielder census counts that state under its own name, and no census panics or drops it

### Requirement: Midfielder movement bands
The seeded batch SHALL report, per match average, and diff against its stored baseline:
- distance covered per midfielder per 90 minutes, against a real band of 10.5 to 12.5 km;
- the share of live play a midfielder spends at or below 2 m/s, against a real band of 45% to 70%;
- state changes per midfielder per minute, against a ceiling of 24.

The first two SHALL be tracked as a diff from the baseline even while they remain out of band. The third SHALL be checked.

#### Scenario: A midfielder's movement is reported against the real bands
- **WHEN** the batch runs
- **THEN** it reports distance per midfielder per 90 minutes and the share of live play he spends at or below 2 m/s, each with its real band and its difference from the stored baseline

#### Scenario: A midfielder's churn is band-checked
- **WHEN** a change raises state changes per midfielder per minute above 24
- **THEN** the batch report shows that statistic out of band and diffs it against the baseline

### Requirement: Forward action census
Under the `match-logs` feature the engine SHALL report, per match, how forwards move, what job each of them has, and how each contact and each run they are part of is decided. The report SHALL include:
- distance covered, the share of time in each speed band and the number of sprints, by position in the front line (central, wide), split by which side has the ball, by live and dead ball, and by state;
- for every forward-tick out of possession, whether his side's assignment named him to close the carrier, whether he was inside the loose-ball election's band, and the state he was in;
- for every forward-tick in possession, the job the attack's assignment gave him (the run in behind, a place in the penalty area, the short outlet, a touchline, none), the state he was in, and whether the player the passer was told to look for in behind was the player in the run-in-behind state;
- for every run in behind, who made it and whether the assignment had named him, what started it, how long he waited on the line and at what speed, how far he was from the referee's offside line, whether the ball was played to him, and how the run ended: he got the ball, he was flagged, his side lost the ball, the run passed to another man, or a rule of the state ended it;
- for every spell in which a forward engages a carrier, the time he spent closing and the time he spent containing, the kind of challenge, and how the spell ended;
- for every header a forward plays, which rule decided it, the state he was in, whether he left the ground, and what it was recorded as; and for every entry to his heading state, the exit it left through;
- for every shot and every pass a forward's state asks for, the site that asked, whether it was taken, and, when it was not, what refused it;
- for a fixed list of situations (a carrier within challenge range of the man for him, a carrier near a man who is not, a loose ball the election gives him, a high ball he has won, the ball lost by his side, the ball won by it, the forward beyond the opposition's last line), the share of forward-ticks, by state, on which the state acted on the situation;
- the share of time a forward spends relaxed, alert, closing, in a duel, getting back, on each attacking job and on the ball, and how often he changes between relaxed and alert;
- for every loose-ball chase a forward's own decision starts, whether his side's election reversed it within ten AI ticks.

Every per-state table in the forward diagnostics SHALL be sized from the state set itself. Adding a forward state SHALL therefore neither crash a census nor drop that state silently from it.

#### Scenario: A forward's distance is split
- **WHEN** the batch runs
- **THEN** the census reports a central forward's and a wide forward's distance per 90 minutes separately, each split by which side has the ball and by live and dead ball

#### Scenario: A run nobody was given is visible
- **WHEN** a forward makes a run in behind while the assignment has named another player, or nobody, for it
- **THEN** the census counts that run as not named

#### Scenario: A run ended by a roll is visible
- **WHEN** a run in behind ends by a rule of the state while his side still has the ball and the run is still his
- **THEN** the census counts it under that ending

#### Scenario: A refused pass is visible
- **WHEN** a forward's state asks for a pass and no pass is played
- **THEN** the census counts the request under the site that made it and names what refused it

#### Scenario: A blind door is visible
- **WHEN** a forward in some state does not act on his side losing the ball
- **THEN** the door table shows that state acting on that situation on less than all of its ticks

#### Scenario: A new forward state is counted
- **WHEN** a forward enters a state whose id is higher than every id that existed before this change
- **THEN** every forward census counts that state under its own name, and no census panics or drops it

### Requirement: Forward movement bands
The seeded batch SHALL report, per match average, and diff against its stored baseline:
- distance covered per forward per 90 minutes, against a real band of 9.5 to 11.5 km;
- the share of live play a forward spends at or below 2 m/s, against a real band of 50% to 75%;
- state changes per forward per minute, against a ceiling of 22.

The first two SHALL be tracked as a diff from the baseline even while they remain out of band. The third SHALL be checked.

#### Scenario: A forward's movement is reported against the real bands
- **WHEN** the batch runs
- **THEN** it reports distance per forward per 90 minutes and the share of live play he spends at or below 2 m/s, each with its real band and its difference from the stored baseline

#### Scenario: A forward's churn is band-checked
- **WHEN** a change raises state changes per forward per minute above 22
- **THEN** the batch report shows that statistic out of band and diffs it against the baseline

### Requirement: A census run plays the formation it names
Every mode of the match harness that builds its own squads SHALL build them in the formation it was asked for, and SHALL print that formation in its report. A mode SHALL NOT print one formation and play another.

#### Scenario: A state census in another formation
- **WHEN** the state-machine census is asked for a formation with three forwards
- **THEN** each side fields three forwards, and the census lists them

#### Scenario: The printed formation is the one played
- **WHEN** any census mode is run with a formation selected
- **THEN** the formation in its header is the formation of the squads it built

### Requirement: Team census
Under the `match-logs` feature the engine SHALL report, per match and per side:
- **the coach:** the plan at kickoff, the commitment reading through the match, every change to the plan with its minute and the moment that prompted it, and the half-time talk;
- **a game-state ledger:** the time the side spent leading, level and trailing, and in each state:
  - its possession share and territory;
  - its shots and xG;
  - the opposition passes it allowed per defensive action;
  - the height of its defensive line, and the length and width of its block;
- **its style:**
  - how high it defended and pressed, how compact and how wide it played, and its possession share;
  - passes per possession and the speed at which it moved the ball toward goal;
  - counter-attacks: possessions won that reach a shot within 15 s;
- **its possessions:** how many it had, their length in passes and in seconds, and how they ended;
- **adherence:** each player's mean reading, and how far each player stood from the plan's place;
- **familiarity:** the side's familiarity at kickoff and after each change of formation.

#### Scenario: A coach's match can be read back
- **WHEN** a match is played under the `match-logs` feature
- **THEN** the report shows each side's plan at kickoff, its commitment by minute, and each change with its minute and moment

#### Scenario: The ledger splits by game state
- **WHEN** a side leads for 30 minutes, is level for 50 and trails for 10
- **THEN** the ledger reports its possession, shots, xG, pressing and line height separately for each of the three states, weighted by those times

### Requirement: Team play bands
The seeded batch SHALL report, as per-match averages, and diff against its stored baseline:
- the correlation of the two sides' goal counts, against a band of −0.08 to +0.08;
- the draw share's surplus over what independent scoring at the batch's own rates would give, against a band of −3 to +3 percentage points;
- a leading side's scoring rate over a trailing side's, before and after the 62nd minute, each against a band of 0.75 to 1.33;
- how much less possession a side leading by one goal holds than when level, against a band of −2 to 10 percentage points;
- the share of go-ahead goals equalised within 15 minutes, and the share of matches in which the lead changes hands, each beside the share that independent scoring at the batch's own rates would give;
- the share of goals scored in each 15-minute period, against the real profile of about 11%, 14%, 16%, 15%, 18% and 26%.

The first four SHALL be checked. The last two SHALL be tracked as a diff from the baseline.

A go-ahead goal is a goal that takes a side from level into the lead. It counts as equalised within 15 minutes when the score is level again within 15 minutes of it, whatever goals come between.

#### Scenario: A draw machine is caught
- **WHEN** a change raises the draw surplus above 3 percentage points, or the goal correlation above +0.08
- **THEN** the batch report shows that line out of band and diffs it against the baseline

#### Scenario: The equaliser is counted from the go-ahead goal
- **WHEN** a side goes ahead in the 30th minute and the score is level again in the 40th, and in another match level again in the 50th
- **THEN** the first go-ahead goal counts as equalised within 15 minutes and the second does not

#### Scenario: A frozen leader is caught
- **WHEN** a change makes leading sides score at less than 0.75 times the trailing sides' rate after the 62nd minute
- **THEN** the batch report shows that line out of band

#### Scenario: The goal profile is tracked
- **WHEN** the batch runs
- **THEN** it reports the share of goals in each 15-minute period beside the real profile, and its difference from the stored baseline

### Requirement: The strength ladder
The harness SHALL contain a seeded batch that plays sides a fixed number of levels apart. The gaps SHALL be 0, 2, 4, 6 and 9 levels, around Premier League mid-table level, and each fixture SHALL be played at both venues.

For every gap it SHALL report:
- the favourite's wins, the draws, and the underdog's wins;
- each side's goals, shots and xG;
- possession.

It SHALL diff these against its stored baseline. It SHALL check the results against these bands:

| Gap | Favourite wins | Draws | Underdog wins |
|---|---|---|---|
| 2 levels | 37%–53% | 19%–31% | 22%–38% |
| 4 levels | 50%–66% | 16%–28% | 12%–28% |
| 6 levels | 62%–78% | 11%–23% | 6%–20% |
| 9 levels | 70%–86% | 7%–19% | 3%–15% |

#### Scenario: The better side does not always win
- **WHEN** the ladder plays 200 fixtures at a 4-level gap
- **THEN** it reports the favourite's wins, the draws and the upsets with their bands, and a share of favourite wins above 66% is shown out of band

#### Scenario: Both venues
- **WHEN** the ladder plays a gap
- **THEN** half of its fixtures have the favourite at home and half away, and the report splits the results by venue

#### Scenario: The team layer's share of a rung
- **WHEN** the ladder is played once on the coaches' plans and once with both plans pinned neutral
- **THEN** the two reports give, rung by rung, the favourite's wins and goal margin with and without the team layer, so the share of the gap it closes can be read beside the contest models' share

### Requirement: A season reads like a division
The harness's season mode SHALL report the champion's points, the last club's points, the spread of points, goals per match and the top scorer's goals. When the clubs span a real top division (levels 12 to 18), it SHALL check the champion's points against a band of 75 to 100 and the last club's points against a band of 15 to 40.

#### Scenario: A division's table
- **WHEN** a 20-club double round-robin season is played with clubs spread over levels 12 to 18
- **THEN** the report shows the champion's and the last club's points with their bands

### Requirement: The harness can pin the coach and the plan
Every mode of the match harness that builds its own squads SHALL be able to do two things for either side:
- give it a coach with a stated temperament;
- fix its plan to stated settings, including one neutral plan shared by both sides.

The mode SHALL print both in its header. Without a pin, each side's coach SHALL be drawn from the population as for a side with no head coach.

#### Scenario: A neutral plan for a formation verdict
- **WHEN** a formation batch is run with both sides pinned to the neutral plan
- **THEN** the header shows the pin, and both sides play the neutral plan for the whole match apart from the commitment reading

#### Scenario: An unpinned run draws its coaches
- **WHEN** a batch is run without a pin
- **THEN** each side in each match is managed by a coach drawn by that match's seed

### Requirement: Delivery census
Under the `match-logs` feature, the engine SHALL follow every cross from its strike to its first contact and to its end. That covers every delivery the crossing decision makes, every other pass booked as a cross, and every corner delivered into the area.

The `stats` report SHALL print the following, by kind of delivery:
- where it was struck: its depth from the goal line, and whether it was struck from inside the width of the penalty area;
- the closer at the strike: on the delivery's line, on the carrier's run, goal-side elsewhere, beaten, or nobody within 5 m;
- its type: floated, whipped, driven across, pulled back, or early;
- its first contact: an attacker's head or feet, a defender's block, header or clearance, the keeper's claim or punch, or nobody;
- its end: completed, a shot or header at goal, cleared, behind for a corner or a goal kick, into touch, or loose;
- its completion rate.

A byline ball SHALL be a delivery struck from inside the width of the penalty area or from within 6 m of the goal line.

The report SHALL also print three source tables:
- corners by source: a shot blocked, a keeper's save, a cross blocked behind, a cross headed or cleared behind, a pull-back or driven ball turned behind, a pass blocked behind, or other;
- throw-ins by what the last toucher did: tackle, block, clearance, header, pass, cross, carry, a player putting it out under pressure, or other;
- own goals by route: a deflected cross or pull-back, a deflected shot that was heading wide, a clearance or header, the keeper, or other.

The touchline census SHALL measure a throw-in that follows another by the gap since the previous throw-in, not by the time since the last touch.

Deliveries a team and the byline share SHALL be printed per level in `levels` and per formation in `stats`.

#### Scenario: A cross blocked behind is a cross-sourced corner
- **WHEN** an open-play cross is blocked by the closer and goes over the goal line
- **THEN** the delivery's first contact is booked as a defender's block, and the corner is counted as a cross blocked behind

#### Scenario: A throw-in is counted once, by its source
- **WHEN** a tackle at the touchline knocks the ball out of play
- **THEN** one throw-in is counted, under tackle

#### Scenario: Back-to-back throw-ins are measured from the previous throw-in
- **WHEN** a throw-in is awarded ten seconds after the previous one, with the ball touched a second before it went out
- **THEN** the touchline census reads a ten-second gap, not one second

#### Scenario: Deliveries are read by level
- **WHEN** `levels` runs
- **THEN** it prints the deliveries a team and the byline share at each level

### Requirement: Wide play bands
The seeded batch SHALL report, as per-match averages, and diff against its stored baseline:
- the byline-ball share of open-play deliveries, against a band of 15% to 35%;
- cross completion, against a band of 18% to 32%;
- own goals a match, against a band of 0.04 to 0.16;
- the share of open-play deliveries blocked;
- headers at goal from open-play crosses;
- corners by source;
- throw-ins by source;
- own goals by route.

The first three SHALL be checked. The rest SHALL be tracked as a diff from the baseline.

#### Scenario: A flank with no byline is caught
- **WHEN** a change takes the byline-ball share of open-play deliveries below 15%
- **THEN** the batch report shows that line out of band and diffs it against the baseline

#### Scenario: Corner sources are tracked
- **WHEN** the batch runs
- **THEN** it reports corners by source beside the stored baseline, so a corner count that holds while its sources shift is visible
