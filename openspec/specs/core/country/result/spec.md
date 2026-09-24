# core/country/result Specification

## Purpose
Owns season-boundary result processing for a country: reputation updates, once-a-season squad-registration enforcement, preseason recovery, end-of-season trophy/promotion/prize distribution, domestic cup awards, and the monthly loan-return/recall cadence.

## Requirements

### Requirement: Country reputation moves from league competitiveness, international success, and transfer activity

A country's reputation SHALL be adjusted each period by three signals: domestic league competitiveness (tighter points spread between top and bottom raises it), the count of high-reputation clubs (approximating international success), and the volume of completed transfers; the result is clamped to 0-10000.

#### Scenario: A country has zero clubs with high team reputation
- **WHEN** international success is calculated and no club's main team reaches the 0.6 reputation threshold
- **THEN** the international-success contribution to reputation change is negative

#### Scenario: A country has completed more than 50 transfers in the reputation window
- **WHEN** transfer-market reputation is calculated with more than 50 completed transfers
- **THEN** the transfer-market contribution to reputation change is at its maximum positive value

### Requirement: Squad registration enforcement fires once per season and produces a durable event

At season start, for every club with a configured foreign-player limit, the excess foreign players on the registered main-team roster SHALL be marked Unregistered and SHALL receive a squad-registration-omitted happiness event.

#### Scenario: A country has no foreign-player limit configured
- **WHEN** squad registration enforcement runs for that country
- **THEN** no players are marked Unregistered and no reserve/youth-squad rosters are touched

### Requirement: Preseason training camps recover match readiness and stamina within fixed bands

During preseason, every non-injured player's match readiness SHALL rise (scaled by the club's training facility quality) up to a ceiling of 20, and stamina SHALL rise toward a ceiling of 20 scaled by natural fitness; injured players are skipped entirely.

#### Scenario: A player is injured during a preseason tick
- **WHEN** preseason training camps process a club's roster
- **THEN** an injured player's match readiness and stamina are left unchanged that tick

### Requirement: Season-end trophy and promotion events are fired once per league table

At season end, for every non-friendly league with a completed final table, the champion SHALL receive a league-title trophy event (or, for a lower-tier league that also promotes, a softened trophy event combined with a promotion event), and every other promoted club SHALL receive a promotion event; grouped competitions crowned by a playoff SHALL NOT fire a table-based title event for the zone/conference topper.

#### Scenario: A lower-tier league's champion also gains promotion
- **WHEN** a tier-2+ league with promotion spots crowns its table-top club
- **THEN** that club's players receive both a softened TrophyWon event (prestige 0.6) and a full PromotionCelebration event

#### Scenario: A league is crowned via a playoff bracket
- **WHEN** a league's settings mark it as playoff-crowned
- **THEN** the table-top finisher does not receive a table-based league-title event; the eventual playoff winner is awarded separately

### Requirement: Prize money and TV revenue are distributed by a top-heavy quadratic curve

End-of-season prize money and merit-based TV revenue SHALL be distributed across a league table using a quadratic decay by finishing position (better position gets disproportionately more), normalized so the shares sum to the full pool; TV revenue additionally splits 50% equally among all clubs and 50% by the same merit curve.

#### Scenario: A league table is processed for season awards
- **WHEN** prize money is distributed across a completed final table
- **THEN** the champion's share of the prize pool is larger than any lower-placed club's share, following the quadratic position-decay curve

### Requirement: Domestic cup winner awards require an appearance in the winning campaign

A domestic cup's winner-fan-out SHALL award the trophy happiness event and reputation impact only to players who actually appeared in a winning-campaign match (per that player's cup competition statistics); unused substitutes and players who never made the squad receive nothing, and involvement scales the event's magnitude by appearance count with an extra bonus for having played in the final itself.

#### Scenario: No player on the winning roster has any recorded cup appearance
- **WHEN** the domestic cup resolves and every eligible check finds zero cup statistics entries
- **THEN** no trophy achievement or player award is applied that tick, and the win is deferred rather than silently dropped

### Requirement: Loan returns process on a monthly cadence and route buyout options correctly

Loan returns SHALL be scanned only around month boundaries (the 1st, or days 28+). An expiring loan with a stored purchase obligation SHALL always execute the buyout; a stored purchase option SHALL execute only when the borrower can afford the fee, the player made at least 10 appearances, and his average rating clears a bar that is lowered when he has expressed a wish to stay.

#### Scenario: A loan carries a binding obligation to buy
- **WHEN** the loan expires and an obligation-to-buy fee is stored
- **THEN** the buyout always executes regardless of the player's appearance count or rating

#### Scenario: A loan carries an option to buy and the borrower cannot afford the fee
- **WHEN** the loan expires, an option (not obligation) is stored, and the borrowing club cannot afford the fee
- **THEN** the option lapses and the player is returned to the parent club instead of being purchased

### Requirement: Warehoused loan surplus can be returned early, independent of loan expiry

A loaned-in player who is settled (at least 90 days into the loan), not among the borrower's top-ranked players at his position group, unused (3 or fewer appearances over that window), and carries no pending permanent-purchase option SHALL be eligible for early return even though his loan has not expired.

#### Scenario: A loaned-in player has played 2 games in 100 days and sits outside the club's positional depth chart
- **WHEN** the monthly loan-return scan runs
- **THEN** that player is flagged for early return as a warehoused surplus loan, separate from any naturally expiring loans that month
