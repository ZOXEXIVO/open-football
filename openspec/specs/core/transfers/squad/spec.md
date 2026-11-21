# core/transfers/squad Specification

## Purpose
Turns a club's roster into what it needs and what it would part with: the recruitment brief built against an objective, the continuous sell list a squad review produces, and the bounded emergency fill when a position group falls critically short.

## Requirements

### Requirement: A recruitment brief targets an objective, not a raw deficit
The system SHALL build a club's transfer needs as a recruitment brief comparing each shirt's target ability (set by the board's ambition and divisional baseline) against the current incumbent, rather than only flagging outright squad holes, and SHALL classify each resulting slot into a tier (transformative, upgrade, or cover) with a distinct budget share and minimum required improvement.

#### Scenario: A transformative slot requires a large improvement and dominates the budget
- **WHEN** a brief slot is classified as tier A (transformative)
- **THEN** it must represent at least an 8-point ability gain, draws up to 60% of the transfer budget, and at most one such slot is briefed per window

#### Scenario: A cover slot has no minimum gain requirement
- **WHEN** a brief slot is classified as tier C (cover)
- **THEN** it carries no minimum ability-gain requirement and is preferentially filled by a loan rather than a permanent signing

### Requirement: Squad review classifies players onto a continuous sell list with a readiness score
The system SHALL score every contracted player against six additive sell motives (peak value, expiring contract, wage relief, cash need, plan surplus, and the player pushing to leave, plus a pathway-driven "mature sale" motive) and SHALL only market a player to outside buyers once his combined score clears a minimum readiness bar, capped at a maximum number of marketed players per club.

#### Scenario: A player below the readiness bar stays privately held
- **WHEN** a player's combined sell-motive score is below 0.35
- **THEN** he is not marketed to buyers even if one or more individual sell motives apply to him

#### Scenario: Marketed roster is capped
- **WHEN** more than 6 players at a club would otherwise clear the readiness bar in the same pass
- **THEN** only the top 6 by score are marketed, sorted by combined readiness

### Requirement: Emergency squad fill applies looser but still bounded eligibility gates
The system SHALL trigger an emergency signing search when a position group falls below its minimum required size (goalkeeper 2, defender 7, midfielder 7, forward 4) or the total senior squad falls below 11 players, and SHALL score emergency candidates against reputation-gap and ability-band criteria that are looser than ordinary recruitment but still enforce a minimum acceptable score.

#### Scenario: Goalkeeper shortage triggers emergency fill
- **WHEN** a club's senior squad has fewer than 2 goalkeepers
- **THEN** an emergency signing slot is opened for the goalkeeper position

#### Scenario: Emergency candidates below the minimum score are rejected
- **WHEN** a candidate's emergency-fill suitability score is below 45.0
- **THEN** he is not selected to fill the emergency slot even if no better candidate is available that pass

### Requirement: A departure opens a vacancy the same day
When a player the club was picking leaves it, by sale, release or the end of a loan, the club SHALL record a vacancy
on that day. The vacancy SHALL hold what he was: the shirt he played, the level and ceiling the staff perceived, his
age, his stronger foot, his planned role, the fee received and the date. The club's recruitment review SHALL read the
vacancy on its next daily pass, whatever day of the week or month it is, and SHALL then decide what the shirt needs.

How much a departure matters SHALL be one continuous reading of the player's importance to this club: his planned
role and the share of football he was taking. A departing cover player SHALL open a vacancy that asks for little.

A vacancy SHALL be closed when a signing fills the shirt, or when the review judges that a player already at the
club holds the shirt at the departed player's level.

#### Scenario: A sale on a Tuesday in August
- **WHEN** a club sells its first-choice striker on a Tuesday inside the transfer window
- **THEN** a search for a striker is open by Wednesday

#### Scenario: The heir is ready
- **WHEN** the sold player's planned heir is perceived at his level
- **THEN** the vacancy is closed by promotion and no search is opened for the shirt

#### Scenario: A squad player leaves
- **WHEN** a club sells its fourth-choice centre-back
- **THEN** the vacancy asks for cover at most, and nothing if the squad still has enough defenders

### Requirement: A vacancy is briefed against the man who left
A search opened by a vacancy SHALL look for a like-for-like replacement: a player for the same shirt, at a level
measured from the departed player's, on the stronger foot the shirt asks for, in an age band the club's philosophy
sets around its own habits. The level sought SHALL be the departed player's, lowered only as far as the club's
standing and money cannot reach it, and never below what the division's standard asks. The search SHALL carry its
own reason, a replacement, distinct from an upgrade, cover or succession.

A club that would not look for the same kind of player SHALL say why in the search itself: a change of formation
that removes the shirt, or a philosophy that replaces a peak-age player with a younger one.

#### Scenario: A star above the division's standard
- **WHEN** a club sells a striker far above its division's usual standard and its next striker is at that standard
- **THEN** the search asks for a striker near the sold player's level, not one at the division's standard

#### Scenario: A left-footed centre-back
- **WHEN** the sold player was the left-sided centre-back of a pair and was left-footed
- **THEN** the search prefers a left-footed centre-back

#### Scenario: A developing club
- **WHEN** a club whose philosophy is to develop and sell loses a 27-year-old
- **THEN** the search looks in a younger age band than he was, and at a level that reaches his within a season or two

#### Scenario: Out of reach
- **WHEN** no player at the sold player's level would join a club of this standing for the money it has
- **THEN** the level sought is the highest the club can reach

### Requirement: A vacancy has first call on the fee it came from
The money a club recycles from a sale SHALL be reserved first for the vacancy that sale opened, up to the cost of a
like-for-like replacement, before it is shared among the club's other needs. A vacancy closed by promotion SHALL
release its reservation to the rest of the budget.

#### Scenario: A club with no other money
- **WHEN** a club with an empty transfer budget sells a player for 80M and recycles half
- **THEN** the search for his replacement can spend up to that half, not a tier's fraction of it

#### Scenario: Closed from within
- **WHEN** the vacancy is closed by promoting the heir
- **THEN** the recycled money is free for the club's other needs

### Requirement: An open search for the same shirt is re-shaped to the vacancy
When a vacancy is recorded for a shirt the club is already searching for, the open search SHALL be re-shaped to the
vacancy: its level, its age band, its money and its urgency. The club SHALL NOT keep the older, smaller search and
drop the new need.

#### Scenario: A cover search becomes a replacement
- **WHEN** a club looking for a cover right-back sells its first-choice right-back
- **THEN** the open search now asks for a first-choice right-back with the vacancy's money

### Requirement: A replacement search is as urgent as the time left
The priority of a vacancy's search SHALL rise continuously with the departed player's importance and as the days
left in the transfer window fall. The search SHALL use the calendar of the club's own country. A vacancy recorded
after the window has closed SHALL be answered from players who can still be signed, and SHALL be carried, with its
reserved money, into the next window.

#### Scenario: Two weeks left
- **WHEN** a first-choice player is sold with fourteen days of the window left
- **THEN** his replacement search is ahead of every search the club has for cover or for the future

#### Scenario: After the deadline
- **WHEN** a first-choice player's sale completes on the last day of the window
- **THEN** the club looks at unattached players at once, and the search and its money are still there when the next window opens
