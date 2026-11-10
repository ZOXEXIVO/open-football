# core/transfers/loan Specification

## Purpose
Prices and gates the loan market end to end: the three-party agreement score, the continuous eligibility discounts a destination is judged on, the passport-based pull toward home, and the seller-side broadcast that cascades an unplaced player to a wider market.

## Requirements

### Requirement: A loan agreement is scored as a continuous product of four willingness factors
The system SHALL price the plausibility of a loan deal as the product of the parent club's willingness to lend, the borrowing club's appetite, the player's consent, and financial affordability — each expressed on a 0 to 1 scale — rather than as a pass/fail checklist, and SHALL discard a pairing whose combined score falls below a minimum floor.

#### Scenario: A pairing below the floor is dropped
- **WHEN** the product of willingness, appetite, consent, and affordability for a loan pairing falls below 0.01
- **THEN** the pairing is excluded from candidate lists as too implausible to represent

#### Scenario: No single hard veto blocks a loan by itself
- **WHEN** one of the four loan-agreement factors (say, borrower appetite) is low but not zero
- **THEN** the overall pairing score is merely reduced, not automatically excluded, unless it falls below the floor

### Requirement: Loan guard checks convert eligibility into continuous discounts, not hard vetoes
The system SHALL express most loan-eligibility checks (minutes-room, overqualification ceiling, foreign quota headroom) as continuous discounts to the loan's attractiveness rather than outright blocks, with a small set of explicit hard exceptions.

#### Scenario: Goalkeeper loan requires undisputed status
- **WHEN** a goalkeeper is being considered for a loan and another goalkeeper at the borrowing club already outranks him by 8 or more ability points
- **THEN** the loan is hard-blocked on the minutes gate, since a goalkeeper loan requires being the undisputed first choice

#### Scenario: Overqualification is discounted rather than blocked, except at the position-group cap
- **WHEN** a loan candidate's ability exceeds the borrower's current best in that position by up to 25 points (or up to 40 for a genuinely developmental loanee)
- **THEN** the loan is tolerated with reduced attractiveness rather than blocked, unless accepting a fourth body in that position group without sufficient ability margin, which is a hard block

### Requirement: Home-country loan destination is determined strictly by passport
The system SHALL determine whether a loan destination counts as "home" for a player using only his nationality/passport country, never his current club's league or spoken language.

#### Scenario: A foreign-league player is still evaluated against his passport country
- **WHEN** a player currently plays abroad but holds a different nationality than his home-mood desires suggest
- **THEN** "home" loan matching is evaluated strictly against his passport country_id, independent of the league he currently plays in

### Requirement: Cross-border loan reputation gates relax for development players
The system SHALL relax the cross-border/cross-region reputation-gap tolerance for a loan when the player is young and development-focused, allowing him to drop further in league standard than a settled player would tolerate, in exchange for guaranteed playing time.

#### Scenario: Development player drops a national tier for minutes
- **WHEN** a player aged 23 or younger, meaningfully below his own club's standard, is offered a loan to a lower-reputation national league with guaranteed minutes
- **THEN** the country-reputation gate that would otherwise block the move is fully lifted for him

#### Scenario: Settled player keeps a tighter tolerance
- **WHEN** an older, established player is considered for the same cross-region loan
- **THEN** he may drop only a fraction of the region-prestige distance a development player would be allowed to drop

### Requirement: Unanswered loan and permanent listings cascade to a wider market over time
The system SHALL widen the pool of clubs a listed player is offered to as time passes without a match, stepping down reputation tiers and discounting price on a fixed cadence, while never fully abandoning a floor of value.

#### Scenario: Seller-side broadcast cascades down a reputation tier weekly
- **WHEN** a loan listing goes seven days without a matching response
- **THEN** the broadcast reach cascades down one reputation tier, floored by the asset's own willingness-to-cascade score rather than reaching zero

#### Scenario: A stale permanent listing discounts and widens after a grace period
- **WHEN** a permanent listing has gone unsold for 21 days
- **THEN** the club begins actively shopping the player itself, and each further unanswered 7-day window cascades the offer one reputation tier further

### Requirement: A loan staged to clear a stranded listing carries the board's wage subsidy
When a club stages a loan as the result of a stranded-listing review, the share of the player's wage the parent club
keeps paying SHALL be the subsidy its board set at that review. The borrower SHALL pay a correspondingly smaller share,
within the same bounds that limit every other loan wage split. A loan staged for any other purpose SHALL keep its
existing per-purpose subsidy.

#### Scenario: A more resolved board pays more of the wage
- **WHEN** two otherwise identical stranded players are loaned to the same borrower, one staged with a higher board subsidy than the other
- **THEN** the borrower pays a smaller share of the higher-subsidy player's wage, and the parent pays the rest

#### Scenario: Other loans are unchanged
- **WHEN** a player is loaned out for development, without any stranded-listing review
- **THEN** the wage split uses the subsidy his loan purpose already implies

### Requirement: A player's consent to a loan is walled by his level floor
The player's consent factor in a loan agreement SHALL be scaled by how far the borrower's division sits under his level
floor:

- unchanged at or above the floor
- falling linearly across the refusal span
- zero a full span under it, so the pairing falls below the agreement floor and is never offered

His age, his months on the market and his own plan already widen the floor, so they SHALL NOT relieve this term a
second time. A destination country he knows nothing about SHALL remain a cost, relieved by resignation and by his plan,
and never a wall.

#### Scenario: Half a span under his floor
- **WHEN** a borrower's division sits half a refusal span under the player's floor
- **THEN** his consent is half what it would be with the same borrower at the floor

#### Scenario: A full span under his floor
- **WHEN** a borrower's division sits a full refusal span or more under the player's floor
- **THEN** his consent is zero, and the pairing is excluded from candidate lists however willing the parent and the
  borrower are

#### Scenario: A strange country is not a wall
- **WHEN** a borrower's division is at or above the player's floor, but in a country he has no familiarity with
- **THEN** his consent is reduced but not zero

### Requirement: A club does not lend a man it holds back

A parent club SHALL hold back from lending three kinds of player at its key-player level:

- a player inside the starting slots there, at any age
- a player past development age who is within reach of its best at his position, whether or not he currently ranks
  among the starters there
- a player who is starting most of the first team's matches, read off the first team's own team sheet and never off a
  season spent on loan elsewhere

Only his own request or his own plan to go out and play SHALL open the door, and then only while he is not starting
most of the matches. A loan listing, a pathway stage or any other decision of the club's own SHALL NOT open it. No route
to a loan SHALL stage, list, approach or agree one for a man the club holds back, at home or abroad.

#### Scenario: A star third in line at a giant

- **WHEN** a 27-year-old at a giant's key level is third in line at his position, has started few recent matches, and
  has not asked to go
- **THEN** his club reads lending him as below the point it entertains the idea, and no loan is agreed for him

#### Scenario: A boy behind a better man

- **WHEN** a 21-year-old at the same level is third in line at his position
- **THEN** a season away remains open to him

#### Scenario: A man well behind the best at his position

- **WHEN** a 27-year-old who only just reaches the club's key level sits well behind its best at his position and is
  not starting
- **THEN** the club reads him as a man who needs football elsewhere

#### Scenario: The club's own stamp is no consent

- **WHEN** a club's loan list, a depth sweep or its pathway names a 27-year-old at its key level who starts most of its
  matches
- **THEN** no loan is staged, listed or agreed for him

#### Scenario: He asked, and he is out of the side

- **WHEN** a player at the key level who is not starting has formally asked to go
- **THEN** a loan is open to him

#### Scenario: He asked, and he is in the side

- **WHEN** a player at the key level who starts most of the matches has asked to go
- **THEN** his club does not lend him, whether the borrower is at home or abroad

### Requirement: Every route to a loan asks whether the parent entertains it

A loan SHALL be staged, listed, approached or agreed only when the parent club's reading of lending him is at or above
the point where it entertains the idea, whichever route raised it: its own pathway, a squad sweep, an unsold listing, a
borrower's approach at home or abroad, or his own request. The reading SHALL count every spell the club has already sent
him on. A man the club is developing (young, and below its level) SHALL need more to justify each further spell. Any
other man SHALL be lent by it at most twice, after which his answer is the market. Lending a man who is not in the
first-team squad SHALL NOT count against the first team's depth. A man the club would not lend SHALL be left to the
routes that keep, sell or release him.

#### Scenario: A third season away for a man it is not developing

- **WHEN** a 26-year-old squad player has been sent out twice by his club and a depth sweep or a borrower would send him again
- **THEN** no loan is staged or agreed, and he stays to be kept, sold or released

#### Scenario: A second spell

- **WHEN** the same player has had one spell and is not playing
- **THEN** a loan remains open to him

#### Scenario: A boy's third spell

- **WHEN** a 21-year-old below his club's level has been sent out twice
- **THEN** a third loan remains open to him, harder to justify than the second

#### Scenario: A reserve the first team does not need

- **WHEN** a reserve-squad player is loan-listed at a club whose first team is thin in his position
- **THEN** his listing reaches the loan market

### Requirement: A contract's clauses read the club that holds the contract

A clause in a loaned player's parent contract SHALL NOT read the squad or the wages of the club he is lent to.

#### Scenario: A loan to a richer club

- **WHEN** a player whose parent contract matches his club's top earner is on loan at a club paying far more
- **THEN** his parent contract is unchanged
