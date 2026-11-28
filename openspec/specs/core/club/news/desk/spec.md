# core/club/news/desk Specification

## Purpose
The news desks each read a slice of club state and file candidate stories — match reports, squad and dugout beats, transfer business, boardroom and balance-sheet news — for the editor to select from, reading institution-level facts from the club's own dated diary rather than inferring them from before/after state.

## Requirements

### Requirement: The club press reads institution-level events from the dated diary, not inferred state
The news desks SHALL read institution-level events (manager dismissals/appointments, severance paid, takeovers, facility changes) only from the club's dated affair diary, rather than inferring them from before/after state comparisons.

#### Scenario: Manager sacked and a caretaker steps up, then a permanent replacement is appointed a month later
- **WHEN** the head-coach identity changes twice within a month for two different reasons
- **THEN** the board desk files a distinct story for the sacking, the caretaker appointment and the later permanent appointment, reading each from its own diary entry instead of conflating the two identity changes into one story

### Requirement: Individual match stories belong to one fixture

For a hat-trick or a substitute's goal contribution, the weekly press SHALL retain the player's contribution, opposing side and final score from the same eligible fixture whenever those facts are recorded. Hat-trick coverage SHALL distinguish victory, draw and defeat. Substitute coverage SHALL distinguish scoring, assisting and doing both. Any claim about minutes played or the goal that decided the game SHALL require evidence of that fact from this fixture.

#### Scenario: Three goals in a draw

- **WHEN** a player scores all three of his side's goals in a 3–3 draw and also plays in a 2–0 victory that week
- **THEN** the hat-trick story SHALL identify the draw, the three goals and that game's opponent.

#### Scenario: Substitute provides an assist in defeat

- **WHEN** a substitute sets up his side's only goal in a 2–1 defeat and scores none himself
- **THEN** his substitute report SHALL describe his assist and the defeat, attributing the goal to him only as its provider.

#### Scenario: Substitute scores and creates in one outing

- **WHEN** the selected substitute appearance records one goal and one assist in a 2–0 win
- **THEN** his substitute report SHALL credit him with both contributions in that win.

#### Scenario: Two qualifying matches in the same week

- **WHEN** a player has qualifying performances of the same kind in two fixtures and there is room for one report
- **THEN** the story SHALL use one fixture consistently, producing the same selection regardless of input order.

### Requirement: Playing-time stories report the player's recorded concern

The squad desk SHALL distinguish a desire for a first-team opportunity from dissatisfaction with playing time. When the first-team desire has recorded evidence of a reserve posting or a perennial backup role, its story SHALL state the supported reason. A specific first-team concern SHALL take precedence over a simultaneous general minutes complaint when both compete for that line.

#### Scenario: A reserve regular wants senior football

- **WHEN** a player has a recent first-team ambition backed by a recorded reserve posting, even if he plays regularly for the reserves
- **THEN** the report SHALL describe the step up he seeks and his reserve role.

#### Scenario: A senior backup wants a start

- **WHEN** the recorded reason for the same ambition is a perennial backup role
- **THEN** the report SHALL describe his bid to break into the starting side.

#### Scenario: A complaint has limited evidence

- **WHEN** only a general playing-time complaint is recorded
- **THEN** the story SHALL confine itself to his desire for more minutes.

#### Scenario: The reason for a first-team ambition is missing

- **WHEN** a first-team ambition is recorded without supporting evidence about his role
- **THEN** the story SHALL report his first-team ambition without assigning a reason.

### Requirement: The parent club recognizes assists on loan

An outfield loanee with at least three league assists in at least three league appearances in the borrower's current season and active spell SHALL qualify for a loan-form candidate identifying the borrower, league assist total and league appearance total. A qualifying goals report or an existing adverse loan verdict SHALL retain precedence. The assists story SHALL use the editor's existing progress-story recurrence with assists as the tracked figure.

#### Scenario: Six appearances as a provider

- **WHEN** an outfield loanee records three assists and no goals in six appearances for the borrower, with no adverse verdict
- **THEN** his parent paper SHALL receive a loan-form candidate about setting up three goals in six appearances.

#### Scenario: A routine appearance is insufficient for another assist report

- **WHEN** the parent paper recently ran the three-assist report and the player makes a seventh appearance without adding an assist
- **THEN** the same assist tally SHALL remain subject to the editor's repeat suppression.
- **AND** a fourth assist SHALL qualify the updated tally for editorial selection.

#### Scenario: A quiet spell

- **WHEN** a loanee has only two assists or fewer than three appearances
- **THEN** the assists alone SHALL yield no loan-form candidate.

### Requirement: Report verifiable scout visits to a player's matches

The weekly paper covering the player's side SHALL be able to file a scout sighting when a visiting club observed that player at a specific completed fixture, the player took the field, and the interest qualifies for public coverage. The report SHALL identify the watching club and fixture opponent. It SHALL distinguish a single sighting, a return by the same club to a different fixture within 28 days, and sightings by two or more distinct clubs in the reporting week. Only a new qualifying visit in the edition's reporting week SHALL trigger a sighting story.

#### Scenario: A scout watches a substitute in a reserve match

- **WHEN** a visiting scout observes a player who comes off the bench against North End and the visit qualifies for coverage
- **THEN** the edition covering the reserve side SHALL be able to report who watched him and against whom, with the scout's name when recorded and resolvable.

#### Scenario: A return visit has a second fixture behind it

- **WHEN** another scout from the same club watches the player in a different match three weeks later
- **THEN** the paper SHALL be able to report that the club returned, even if the first visit did not make the paper.
- **AND** two observers from that club at a single fixture SHALL count as one club visit.

#### Scenario: Two clubs send scouts

- **WHEN** scouts from two different clubs each make a qualifying visit to watch the player that week
- **THEN** the desk SHALL combine the sightings into one candidate naming both clubs and attributing each visit to its own fixture.

#### Scenario: The paperwork outlives the sighting

- **WHEN** the player is an unused substitute, his participation is unknown, or the only new activity is a planned visit or a desk review
- **THEN** that activity SHALL yield no claim of a scout watching him play, even when his scouting file already contains earlier visits.

### Requirement: Transfer rumours reflect their source and status

The weekly rumour desk SHALL distinguish a press link, an agent sounding out interest, and verified match attendance. It SHALL name a suitor only when the evidence for that particular story identifies the club. Reporting a bid, negotiations or an agreement SHALL require the corresponding recorded development. A stronger confirmed development for the same player and suitor SHALL take the interest line ahead of a scouting sighting in the same edition.

#### Scenario: The agent sounds out a move

- **WHEN** the recorded source is the agent sounding out potential interest and there is no recorded bid
- **THEN** the story SHALL describe the agent's activity without presenting a club offer, negotiations or a fee as fact.

#### Scenario: An unattributed press link circulates

- **WHEN** the player is linked with a move in the local press and the rumour identifies no club
- **THEN** the story SHALL attribute the link to the local press and leave the suitor unnamed, even if a scouting file exists.

#### Scenario: A bid is rejected after the visit

- **WHEN** the same club's verified sighting and its subsequently rejected bid qualify for the weekly interest line
- **THEN** the rejected bid SHALL lead that line and the sighting SHALL NOT produce a second interest story for the same player.
