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

### Requirement: Preseason training camps rebuild match readiness only

During preseason, the country SHALL give every club player one off-season camp day per tick, scaled by the club's
training-facility quality. A camp day SHALL raise a non-injured player's match readiness toward a ceiling of 20. It
SHALL NOT change stamina or any other technical, mental, physical or goalkeeping attribute; those change only through
the development tick and the club's training sessions. Injured players SHALL be skipped entirely.

#### Scenario: A player is injured during a preseason tick
- **WHEN** preseason training camps process a club's roster
- **THEN** an injured player's match readiness and stamina are left unchanged that tick

#### Scenario: A fit player's summer in camp
- **WHEN** preseason camps run every off-season day for a fit player
- **THEN** his match readiness rises with facility quality, and his stamina, concentration, composure, decisions, first
  touch, passing and technique are not raised by the camps

### Requirement: Season-end trophy and promotion events are fired once per league table

At season end, every non-friendly league with a completed final table SHALL award a league-title trophy event to its
champion. The title SHALL be softened in a league that sends sides up to a division above it.

Promotion events (a promotion achievement and a PromotionCelebration) SHALL go to exactly the sides the season-end swap
moves up into a higher division, and to no other side. None of these SHALL receive one:

- a reserve side barred from going up
- a side whose group was allotted fewer places than its own promotion places
- every side of a league that no division relegates into

Grouped competitions crowned by a playoff SHALL NOT fire a table-based title event for the zone/conference topper.

#### Scenario: A lower-tier league's champion also gains promotion
- **WHEN** a tier-2+ league crowns a table-top club that the swap moves up
- **THEN** that club's players receive both a softened TrophyWon event (prestige 0.6) and a full PromotionCelebration event

#### Scenario: A reserve side tops a lower-tier table
- **WHEN** a B team whose first team plays in the tier directly above wins a tier-2 league that promotes two sides
- **THEN** the B team's players receive the softened TrophyWon event but no PromotionCelebration
- **AND** the two sides that actually go up in its place each receive a promotion achievement and a
  PromotionCelebration event

#### Scenario: A group allotted fewer places than it declares
- **WHEN** a division relegating three sits above two groups that each declare two promotion places
- **THEN** the second group's runner-up, who stays down, receives no promotion event

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

Loan returns SHALL be scanned only around month boundaries (the 1st, or days 28+). An expiring loan with a stored
purchase obligation SHALL always execute the buyout.

A borrower SHALL exercise a stored purchase option only when all three hold:

- it can afford the fee
- the player made at least 10 appearances
- his average rating clears a bar, which is lowered when he has expressed a wish to stay

An exercised option SHALL then need the player's own signature. He SHALL appraise a permanent deal at the borrower as
he would any personal terms:

- his loan squad role is the promised role
- the borrower's division is weighed against his level floor
- the club that owns him is the club he would leave

When he refuses:

- the option SHALL lapse
- he SHALL return to his parent club like any other expiring loanee
- the refusal and its cause SHALL be recorded as a decision in his own name

A buyout he signs SHALL count as a move he chose. So SHALL an executed obligation, which he agreed to with the loan. He
SHALL NOT remember either as being sold against his will.

#### Scenario: A loan carries a binding obligation to buy
- **WHEN** the loan expires and an obligation-to-buy fee is stored
- **THEN** the buyout always executes regardless of the player's appearance count or rating

#### Scenario: A loan carries an option to buy and the borrower cannot afford the fee
- **WHEN** the loan expires, an option (not obligation) is stored, and the borrowing club cannot afford the fee
- **THEN** the option lapses and the player is returned to the parent club instead of being purchased

#### Scenario: An international declines the option of a third-tier club
- **WHEN** a fifty-cap international owned by a top-flight giant ends a season of starts at a third-tier borrower,
  which exercises its option
- **THEN** the option lapses, he returns to the giant, and his decision record shows the declined buyout attributed to
  him

#### Scenario: The same option is signed in his own division
- **WHEN** the same season and the same option are at a borrower in his parent club's division
- **THEN** he signs, ownership passes to the borrower, and he stays where he played

#### Scenario: A signed buyout is not a sale against his will
- **WHEN** a buyout he signed completes
- **THEN** he carries no memory of having been sold against his will by his parent club

#### Scenario: An executed obligation is not a sale against his will
- **WHEN** a loan's obligation to buy executes at a borrower under his floor
- **THEN** ownership passes to the borrower, and he carries no memory of having been sold against his will by his
  parent club

### Requirement: Warehoused loan surplus can be returned early, independent of loan expiry

A loaned-in player who is settled (at least 90 days into the loan), not among the borrower's top-ranked players at his position group, unused (3 or fewer appearances over that window), and carries no pending permanent-purchase option SHALL be eligible for early return even though his loan has not expired.

#### Scenario: A loaned-in player has played 2 games in 100 days and sits outside the club's positional depth chart
- **WHEN** the monthly loan-return scan runs
- **THEN** that player is flagged for early return as a warehoused surplus loan, separate from any naturally expiring loans that month

### Requirement: Season-end promotion and relegation keep every division at constant size

Once every non-friendly league in the country has finished its season, the system SHALL swap sides across each
non-split league's lower boundary. It SHALL use the frozen final tables and the places the league ladder allots to that
boundary.

- The worst-placed sides of the upper league SHALL go down.
- Each lower league SHALL send up its top eligible sides, up to its allotted places.
- Promoted sides SHALL be ranked champions-first across the lower leagues: every champion before any runner-up.
- Relegated and promoted counts SHALL always be equal. When either side has fewer candidates than allotted, both SHALL
  be cut to the smaller count, dropping the lowest-ranked promotions first.
- Each lower league SHALL take back exactly as many relegated sides as it sent up. Relegated sides SHALL be dealt
  worst-first, one per lower league in turn in league-id order, skipping leagues whose vacancies are filled.

#### Scenario: A single division above two regional groups
- **WHEN** a six-team division relegating four sits above two groups of four that promote two each
- **THEN** the top two of both groups go up
- **AND** the bottom four go down worst-first into the first group, the second group, the first group and the second
  group, and the divisions end at six, four and four sides

#### Scenario: Ranked groups inside one tier
- **WHEN** a Gold group relegates two into a Silver group of the same tier, and Silver relegates four into four regional
  zones that promote one each
- **THEN** two sides swap each way between Gold and Silver, Silver's bottom four go one into each zone, every zone
  champion goes up into Silver, and every division keeps its size

### Requirement: A reserve side is never promoted into its first team's tier

A club's non-first team that plays in a league (B team, reserve, "2" side, U23, Jong side) SHALL NOT be promoted into a
league whose tier is the same as, or higher than, the tier its club's first team plays in. When such a side finishes
inside the promotion places, it SHALL stay where it is and the next eligible side in that table SHALL go up in its
place. A reserve side MAY still be relegated.

#### Scenario: A B team tops the division below its first team
- **WHEN** a club's B team finishes first in tier 2 while its first team plays in tier 1, and tier 2 promotes one side
- **THEN** the B team stays in tier 2, the tier-2 runner-up is promoted, and the bottom side of tier 1 is relegated in
  exchange

#### Scenario: A reserve side two tiers below its first team
- **WHEN** a reserve side tops tier 3 while its first team plays in tier 1
- **THEN** it is promoted into tier 2 like any other side

### Requirement: A team in a senior division moves only on its own result

A club's team that plays in a non-friendly division SHALL change division at the season-end swap only when that team is
itself promoted or relegated. It SHALL end the swap in the division it moved into.

When a club's first team changes division, only the club's sub-teams that play in youth or friendly competitions SHALL
follow it, into the matching youth or friendly competition of the first team's new division.

#### Scenario: A promoted B team stays promoted
- **WHEN** a club's B team is promoted from tier 3 to tier 2 while its first team stays in tier 1
- **THEN** after the swap the B team plays in the tier-2 division, and that division has as many sides as it had before
  the swap

#### Scenario: A first team's relegation leaves its B team where it plays
- **WHEN** a club's first team is relegated from tier 1 to tier 2 while its B team plays in tier 3
- **THEN** the B team still plays in tier 3, and the club's youth sides follow the first team to the youth
  competitions matching tier 2

### Requirement: Parachute entitlements follow first-team tier changes, not group swaps

When a season-end swap moves a club's first team across a tier boundary:

- a club whose first team was relegated SHALL start a parachute entitlement from the tier it left, at season zero,
  replacing any running entitlement
- a club whose first team was promoted SHALL lose any parachute entitlement

When the swap is between ranked groups of the same tier, no club's parachute entitlement SHALL change.

#### Scenario: Dropping from Gold to Silver inside one tier
- **WHEN** a Gold first team whose club carries a parachute from tier 2, one season elapsed, is relegated into Silver
  of the same tier
- **THEN** the entitlement is still from tier 2 with one season elapsed, and the club of the Silver side promoted in
  its place gains none

#### Scenario: Dropping out of the tier
- **WHEN** a Silver first team in tier 3 is relegated into a tier-4 zone
- **THEN** its club starts a parachute entitlement from tier 3 at season zero
