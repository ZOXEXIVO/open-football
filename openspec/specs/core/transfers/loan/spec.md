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
