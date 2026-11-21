# core/match/squad/selection Specification

## Purpose
Determines the matchday squad for a fixture — eligibility, the starting XI, the bench, and how the opponent's known threat profile shapes those choices.

## Requirements

### Requirement: Matchday squad selection — eligibility
For a given fixture, the system SHALL determine each player's eligibility to be selected from hard eligibility blocks
that exclude him and soft eligibility limits (e.g. returning from injury) that penalize rather than exclude him.

The hard blocks SHALL be:
- injury;
- international duty;
- a suspension that applies in this fixture's competition;
- a cup tie: he has already appeared for another club in this season's edition of the competition;
- his loan: a borrowed player SHALL NOT be selected against the club that owns him;
- an agreed move: a player whose club has accepted a bid for him, or whose transfer is agreed, SHALL NOT be selected.

The agreed-move block SHALL give way only when the side could not otherwise name eleven players with a goalkeeper
among them. Every block SHALL be read from the party that owns the fact, and SHALL apply alike in league, domestic cup
and continental fixtures.

A player's transfer-market status short of an agreed move (listed, requested, unhappy) SHALL NOT make him ineligible.

#### Scenario: Injured player cannot be selected
- **WHEN** a player is currently injured
- **THEN** that player is hard-blocked from selection for the fixture regardless of other factors

#### Scenario: Player returning from injury can still be selected under pressure
- **WHEN** a player is in a recovery/return-to-fitness period but not hard-blocked
- **THEN** that player remains selectable with a penalty applied to their selection score rather than being excluded outright

#### Scenario: Cup-tied player excluded from a specific competition
- **WHEN** a player is cup-tied to a different club for the competition of the current fixture
- **THEN** that player is excluded from selection for that fixture even if eligible for other competitions

#### Scenario: A loanee does not face his parent club
- **WHEN** a club's fixture is against the club that owns one of its borrowed players
- **THEN** that player is not named in the eleven or on the bench, and he is selectable again for the next fixture

#### Scenario: A sending-off in Europe does not cost a league match
- **WHEN** a player is sent off in a continental tie and his club's next fixture is a league match
- **THEN** he is eligible for the league match and is blocked from his club's next continental tie

#### Scenario: An agreed move in a must-win match
- **WHEN** a club has accepted a bid for a starter and its next fixture is a must-win league match with a full squad available
- **THEN** he is not named in the eleven or on the bench

#### Scenario: An agreed move at a club that cannot name eleven
- **WHEN** the same club has only ten other eligible outfield players
- **THEN** he is selectable for that fixture

#### Scenario: A listed player stays selectable
- **WHEN** a player is transfer-listed and no bid for him has been accepted
- **THEN** he is eligible exactly as an unlisted player is

### Requirement: Matchday squad selection — fixture context and starting XI
The system SHALL choose the starting XI as one assignment of players to the shirts of the formation. The assignment
SHALL maximise, over the whole XI, each player's fit for his shirt today together with the football the manager's
plan owes him.

Fit SHALL be the coach's own reading of the player for that shirt: position familiarity, perceived quality, form,
match readiness, physical risk, how he combines with the men beside him, and the opponent's known threats. The
responsible coach's selection tendencies (rotation discipline, star favoritism, academy trust, tactical flexibility,
medical caution) SHALL shape that reading and how far he uses the room a fixture leaves.

The weight of the football owed SHALL be one continuous quantity, the room the fixture leaves. Room SHALL fall as the
fixture's stakes for this side rise, reaching nothing at a must-win, and SHALL rise when a fixture that matters more
to the club lies within a few days in any competition. The system SHALL NOT switch between named selection policies
at thresholds of importance. It SHALL NOT add a term keyed to the competition's name, the stage of a bracket, a
squad-status label or an age bracket.

The system SHALL classify the upcoming fixture's type and the coach's tactical objective (e.g. routine league match,
title race, relegation six-pointer, derby, cup round, continental knockout, friendly; protect a lead, chase a game,
develop players) from the competition and stakes context. The same context SHALL be built for league, domestic cup
and continental fixtures: the stakes for this side, the club's week, the opponent, the club's philosophy, the
goalkeeping department's word and the competition's rules.

A player pinned by the manager SHALL start whenever he is eligible.

#### Scenario: Cup final classified distinctly from an early cup round
- **WHEN** the fixture is the last round of a domestic cup competition
- **THEN** it is classified as a cup final rather than a cup early-round or cup-knockout fixture, and selection weighs it accordingly

#### Scenario: Fixture congestion increases rotation pressure
- **WHEN** the selecting club has a competitive fixture in any competition within a few days of the current one
- **THEN** selection has more room to rotate than for a club with no nearby fixtures, in proportion to how much that other fixture matters against this one

#### Scenario: A cautious coach rotates less aggressively
- **WHEN** the responsible coach's profile reflects high conservatism and rotation discipline
- **THEN** the selection scoring favors sticking with established first-choice players over an otherwise-similar coach with low rotation discipline

#### Scenario: A must-win leaves no room
- **WHEN** a fixture's stakes for the selecting side are at their highest and a fit first-choice player is well ahead of his planned share
- **THEN** he starts, and no player starts because football is owed to him

#### Scenario: Two near-equal fixtures are picked alike
- **WHEN** two fixtures differ in stakes by a small amount and nothing else differs
- **THEN** the two starting XIs differ by at most one player

#### Scenario: The same club is read the same way in Europe
- **WHEN** a club plays a continental tie
- **THEN** its selection reads the same philosophy, the same declared goalkeeping order and the same plan as in its league fixture that week

### Requirement: Matchday squad selection — opponent-aware threat profile
When the opponent's roster is known, the system SHALL derive a continuous opponent threat profile (pace, aerial, pressing intensity, low-block likelihood, set-piece threat, left/right wide threat, central overload) from that roster's actual player attributes and tactical shape rather than from a fixed or generic opponent assumption.

#### Scenario: Strong aerial opponent raises the aerial-threat signal
- **WHEN** the opponent's best few players in heading and jumping attributes are materially above league-average
- **THEN** the opponent's aerial-threat signal used by selection is elevated accordingly

#### Scenario: No opponent roster available falls back to neutral
- **WHEN** the opponent roster is empty or unavailable at selection time
- **THEN** the opponent threat profile defaults to a neutral, balanced reading rather than to a zero or undefined threat

### Requirement: Matchday squad selection — bench composition
The system SHALL select a bench of the size the fixture's competition allows from the remaining eligible,
non-starting players.

The bench SHALL name exactly one goalkeeper whenever one is available. It SHALL name a second only when the
competition's bench has twelve places or more.

The outfield places SHALL first cover the starting XI: each line of the team on the pitch (central defence, the wide
defenders, central midfield, the wide and attacking players, the forward line) SHALL have a substitute the coach
would use there before any place is filled on quality alone. The remaining places SHALL consider tactical balance,
the planned match scenarios and the football the plan owes. When the fixture leaves room, one place SHALL go to a
player on a development role in the plan who is eligible.

#### Scenario: Position coverage on the bench
- **WHEN** the starting XI is thin at a given position group relative to the rest of the squad
- **THEN** bench selection favors including cover for that position group over an otherwise higher-rated player at an already well-covered position

#### Scenario: A bench of seven never names two keepers
- **WHEN** a competition allows seven substitutes and the club has three fit goalkeepers
- **THEN** the bench names one goalkeeper and six outfield players

#### Scenario: A larger bench where the competition allows it
- **WHEN** a competition allows nine substitutes and the club has twenty fit outfield players
- **THEN** the bench names nine players

#### Scenario: A prospect's seat on a quiet night
- **WHEN** a fixture leaves a lot of room and the plan holds a fit prospect on a development role
- **THEN** he is named on the bench ahead of a senior player who covers a line already covered

#### Scenario: No seat on a night that matters
- **WHEN** the fixture is a must-win
- **THEN** every bench place is filled for the match in front of the team

### Requirement: Rotation follows football owed, not the competition's name
How much of a first-choice XI a club fields SHALL follow from three readings only: the stakes of the fixture for that
club, the club's week across its competitions, and how far each player stands from the share of football the plan
gives him. A domestic cup tie, a continental tie and a league match with the same three readings SHALL be picked
alike.

#### Scenario: A small club hosts a giant in the cup
- **WHEN** a lower-division club meets a far stronger club in a domestic cup round with no other fixture that week
- **THEN** it fields its strongest available XI

#### Scenario: A giant visits a small club in an early round
- **WHEN** a strong club with a league match three days later meets a far weaker club in an early cup round
- **THEN** most of its XI are players behind their planned share, and a first-choice player who is himself behind his share may still start

#### Scenario: The league match before a decisive European night
- **WHEN** a club plays a routine league match three days before the second leg of a continental tie that is level
- **THEN** it rests several first-choice players in the league match and fields its strongest XI in the tie

#### Scenario: A group match with nothing on it
- **WHEN** a club's place in a continental group is already settled before its last group match
- **THEN** it picks that match as it would a league match with nothing on it

#### Scenario: A full week between league matches
- **WHEN** a club with no other competition plays league matches a week apart
- **THEN** its XI changes little from one match to the next

### Requirement: The goalkeeper is the declared one
The starting goalkeeper SHALL be the keeper the club's goalkeeping order declares for the fixture. That is the
declared number one whenever he is eligible, except in:
- a fixture of the competition declared as the deputy's, up to the round the declaration reaches;
- a fixture for which the goalkeeping department holds a live nomination that the manager acts on;
- a fixture from which the department advises resting a shaken number one and the manager acts on it.

When the declared keeper is not eligible, the next keeper in the declared order SHALL play. The system SHALL NOT
choose a goalkeeper by comparing per-match scores of fatigue, football owed, market status or loan fees, and a
congested week SHALL NOT rest a goalkeeper. A club with no declared order SHALL field its best eligible goalkeeper by
what its coaches can observe. A goalkeeper pinned by the manager SHALL play.

#### Scenario: A routine league match in a crowded week
- **WHEN** a club plays three matches in eight days and its number one is fit
- **THEN** he starts all three unless one of them belongs to the deputy's competition

#### Scenario: The deputy's cup
- **WHEN** the domestic cup is declared as the deputy's up to the quarter-final and the club plays a third-round tie
- **THEN** the deputy starts, whoever the opponent is

#### Scenario: Past the declared round
- **WHEN** the same club reaches the semi-final
- **THEN** the number one starts

#### Scenario: The number one is injured
- **WHEN** the declared number one is injured
- **THEN** the declared deputy starts every fixture until he returns

### Requirement: Selection reads only what a coach can observe
No part of matchday selection SHALL read a player's hidden current ability or hidden potential. The goalkeeper order,
the reading of who is being groomed behind whom, a prospect's credibility and every emergency fill SHALL use the
level and ceiling the club's staff perceive.

#### Scenario: Two keepers who look alike
- **WHEN** two goalkeepers have the same visible attributes, form and record, and different hidden ability
- **THEN** selection treats them alike

#### Scenario: An emergency fill
- **WHEN** a side must fill a shirt with nobody suited to it
- **THEN** the player chosen is the best by perceived level, not by hidden ability

### Requirement: Every side of a club picks from a plan
A club's B, second, reserve and youth sides SHALL pick their teams by the same mechanism as the first team: fit for
the shirt and football owed under that side's plan, within the room the fixture leaves. What differs between sides
SHALL be one continuous weight, how far the side exists to develop players, which sizes the plan's shares toward the
players the staff believe in and widens the room an ordinary fixture leaves.

#### Scenario: A B team in a real league
- **WHEN** a B team in a senior league holds a 19-year-old the staff rate highly and a 26-year-old who is slightly better today
- **THEN** over a season the 19-year-old starts more of its matches

#### Scenario: A B team's decider
- **WHEN** the same B team plays a match that decides its promotion
- **THEN** it fields its strongest available XI

#### Scenario: A youth league fixture
- **WHEN** an under-18 side plays an ordinary league fixture
- **THEN** its minutes are shared across the squad by how much the club believes in each player, as before

### Requirement: A season of team sheets reads like football
Over a stubbed season, the first teams of top-division leagues SHALL produce team sheets inside the reference bands
the selection census carries.

#### Scenario: The settled league XI is kept
- **WHEN** the census reads XI changes from one league match to the next
- **THEN** the mean is between 1.5 and 3.0

#### Scenario: Cup ties are rotated, not replaced
- **WHEN** the census reads XI changes going into a domestic cup tie across all rounds and clubs
- **THEN** the mean is between 4.0 and 6.5

#### Scenario: European weeks are rotated
- **WHEN** the census reads XI changes after four days' rest or fewer
- **THEN** the mean is between 3.0 and 5.0

#### Scenario: Football is spread through the squad
- **WHEN** the census reads the share of a club's league starts taken by its eleven most-used players
- **THEN** the mean is between 68% and 77%

#### Scenario: Benches
- **WHEN** the census reads benches of nine or fewer
- **THEN** fewer than 1% name two goalkeepers

#### Scenario: Goalkeepers
- **WHEN** the census reads every starting XI of the season
- **THEN** no side starts an outfield player in goal while its club has a fit goalkeeper free that day
