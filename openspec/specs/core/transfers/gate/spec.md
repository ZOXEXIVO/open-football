# core/transfers/gate Specification

## Purpose
Decides whether a prospective move may happen at all — the staged plausibility model, the player's own appraisal of an offer, and the squad-fit and quota checks a buyer must clear.

## Requirements

### Requirement: Staged eligibility gate for every move
The system SHALL evaluate every prospective transfer or loan through an ordered sequence of eligibility stages (from "may be privately scouted" through "may be shortlisted," "may show public interest," "may open negotiation," "may agree personal terms," to "may complete") and SHALL only allow a downstream action once the move has cleared every stage up to and including the one that action requires.

Every route by which a club opens, keeps or advances attention on another club's player SHALL read this one gate, and SHALL name the stage its action needs:

- keeping a file on him, a place on a watchlist, or a staff recommendation needs "may be privately scouted";
- a place on a shortlist, or a transfer request opened for him, needs "may be shortlisted";
- interest shown in public needs "may show public interest";
- talks with his club need "may open negotiation".

A route that is filling a shirt in the window it is shopping in MAY ask for a later stage than its action needs. No route SHALL decide the question by a rule of its own, and no route SHALL be given a yes or a no in place of the stage the move reached and what stopped it.

The gate SHALL read the same club and the same player the same way on every route. What a route cannot supply SHALL NOT read as favourable:

- a move's market reach SHALL always be the club's own reach into that market;
- a player who cannot be read SHALL NOT be pursued.

#### Scenario: Move blocked before negotiation
- **WHEN** a club wants to open formal negotiations for a player who has not cleared the negotiation-eligibility stage
- **THEN** the system refuses to create a negotiation and reports the blocking reason instead

#### Scenario: A move that may be watched but not approached
- **WHEN** a club's year-round watch reads a key player his club will not release, who would play in the club's division, whom the club could pay, and whose market the club works
- **THEN** the move reaches "may be privately scouted" and the club may keep a file on him
- **AND** no shortlist place, public interest or negotiation follows

#### Scenario: The same pair reads the same on every route
- **WHEN** the request scan, the breakout watch and the staff's advice each consider the same player for the same club on the same day
- **THEN** each is given the same stage and the same blocking reason

#### Scenario: A player who cannot be read is not pursued
- **WHEN** a foreign name in a club's memory no longer resolves to a readable player
- **THEN** no recommendation, transfer request or shortlist place is made for him

### Requirement: Player availability is graded, not boolean
The system SHALL classify how available a player is on a graded scale (none, soft, real, forced) rather than as a single yes/no flag, driven by signals such as a triggered release clause, a formal transfer request, a matching listing, near-expiring contract with an affordable fee, financial distress at the selling club, or informal unhappiness/agent circulation.

#### Scenario: Release clause forces availability
- **WHEN** a buyer triggers a player's release clause
- **THEN** the player's availability is graded "forced," bypassing the sporting-importance and reputation-drop checks that would otherwise apply

#### Scenario: Real availability from a formal transfer request
- **WHEN** a player has formally requested a transfer and a club matching the request's deal type approaches
- **THEN** the player's availability is graded "real," which unlocks the importance/step-down hard gate and waives the wage-floor requirement for that approach

### Requirement: Hard reject rules protect important players from unsolicited poaching
The system SHALL reject an unsolicited approach for an important player at a materially stronger club when no availability signal is present, using continuous thresholds on player importance and the sporting/reputation gap rather than fixed categories.

#### Scenario: Important player refuses an unsolicited step-down
- **WHEN** a club with importance ≥0.78 to his current side is approached unsolicited by a club representing a sporting drop of ≥0.16, with no availability signal open
- **THEN** the approach is hard-rejected before negotiation can start

#### Scenario: Prime-age domestic starter resists a smaller step-down
- **WHEN** an important player aged 23–30 is approached unsolicited for a domestic move representing a sporting drop of ≥0.19, with no opening signal
- **THEN** the approach is hard-rejected

### Requirement: Registered-foreigner quota limits, but rarely fully blocks, a signing
The system SHALL treat a buying club's registered-foreigner quota as a continuous factor that discourages recruiting further foreign players as it fills, and SHALL only hard-block a foreign signing outright when the buyer is already at or over its quota.

#### Scenario: Quota fully consumed blocks a foreign signing
- **WHEN** a club is at or over its registered-foreigner quota and the candidate is a foreign player
- **THEN** the signing is blocked outright

#### Scenario: Quota nearly full only discourages
- **WHEN** a club has one registration slot remaining under its foreign quota
- **THEN** the club may still complete a foreign signing, though the fit assessment reads reduced room

### Requirement: Squad fit exempts promising youth and short-handed groups
The system SHALL treat a candidate as squad-surplus (blocking recruitment) when his assessed ability sits well below the squad average for his position group or he would rank outside the group's depth cap, but SHALL exempt promising young players and SHALL bypass the fit check when the position group is short-handed.

#### Scenario: Promising youth is exempt from the surplus check
- **WHEN** a candidate is 23 or younger with believed potential more than 10 points above his believed current ability
- **THEN** he is not treated as squad-surplus, unless the club's development quota for that position is already full

#### Scenario: Short-handed group bypasses the fit gate
- **WHEN** a position group's current size plus one signing would still fall below the club's depth cap
- **THEN** the squad-fit check is bypassed because the club needs bodies regardless of surplus concerns

### Requirement: Player-side appraisal weighs money against sporting, role, and personal factors
The system SHALL evaluate whether a player accepts a move as a single utility score summed across weighted axes, and
SHALL accept the move only when that utility, plus a fixed per-negotiation random disposition, exceeds zero. The axes
are:

- money
- sporting trajectory
- promised role
- destination prestige
- home pull
- pressure to leave
- attachment to the current club
- personal circumstance and memory
- fit with his own career plan
- the level floor
- his career: what the move does for the dream he holds

The level-floor axis SHALL be zero or negative:

- Its penalty SHALL grow with how far the division he would play in sits under his level floor.
- The penalty SHALL be discounted by the weight money carries for him at that point of his career. A payday late in a
  career, or on an expiring deal, can buy a division a player in his prime refuses.
- When either the destination's division or his floor cannot be read, the axis SHALL be zero.
- When a refusal's cause is named, the level-floor axis SHALL count toward a sporting step-down.

The career axis SHALL read his dream on the same footing for every player and every path, domestic or foreign:

- A permanent move toward the level his dream names SHALL pay, and a move away from it SHALL cost, in proportion to his
  conviction. Nothing SHALL be paid for going beyond that level.
- An offer from the club his dream names SHALL pay in full.
- A cost on this axis SHALL be discounted by the weight money carries for him, as on the level-floor axis.
- A loan SHALL contribute nothing on this axis, because a loan does not take him away from what he is reaching for.
- With no dream to read, or an offer whose level cannot be read, the axis SHALL be zero.
- When it is what cost him most, a refusal SHALL name his career as the cause.

A goal the dream feeds, such as going home or staying, SHALL reach the appraisal only through its existing axis, and
SHALL NOT be counted again on this one.

#### Scenario: A raised offer that reaches the player's number is guaranteed accepted
- **WHEN** a club raises an offer so that the computed utility plus the player's disposition exceeds zero
- **THEN** the player accepts; the acceptance is not re-rolled at the higher offer

#### Scenario: A sporting step-down needs a compensating wage
- **WHEN** a move represents a sporting step-down for an ambitious player
- **THEN** the money axis must be raised enough to offset the sporting-axis penalty before total utility can turn positive

#### Scenario: A loan a division under his floor is refused whatever the push
- **WHEN** a 27-year-old international at a top-flight giant, listed, unhappy, asking to leave and promised a
  first-team shirt, is offered a loan into a division a full refusal span under his floor
- **THEN** he refuses, and the cause is a sporting step-down
- **AND** the same offer from a club one division nearer, at or above his floor, is accepted

#### Scenario: Late in a career the floor is for sale
- **WHEN** a 33-year-old with the same floor, on an expiring deal, is offered a large wage and a key-player role in
  that same division
- **THEN** the level-floor penalty is small and he accepts
- **AND** a 27-year-old offered a smaller raise there still refuses

#### Scenario: An unread division objects to nothing
- **WHEN** the destination's division or the player's floor cannot be read
- **THEN** the level-floor axis contributes nothing to his utility

#### Scenario: A star at a giant does not sign for a second-tier club
- **WHEN** a 22-year-old key player who has arrived at one of the world's biggest clubs is listed by it and offered a
  large wage and a key role by a club in a division well under his floor
- **THEN** he refuses, and the cause is a sporting step-down or his career

#### Scenario: A climber takes the step
- **WHEN** two otherwise identical players are offered the same move to a bigger club at the same wage, and one has
  strong conviction in a dream above his club while the other wants only to make a career
- **THEN** the career axis is positive for the climber and zero for the other, and the climber is the likelier to accept

#### Scenario: His club calls
- **WHEN** a player whose dream names a club is offered terms by that club at less than he earns
- **THEN** the career axis pays in full, and he accepts a cut he would refuse elsewhere

#### Scenario: A loan leaves the dream alone
- **WHEN** a 20-year-old reaching for the top is offered a loan to a smaller club
- **THEN** the career axis contributes nothing

#### Scenario: A move that serves nothing he wants
- **WHEN** a move's level is below where he plays and the career axis is the largest cost, and he refuses
- **THEN** the refusal names his career as the cause

### Requirement: Reservation wage is derived, not separately authored
The system SHALL derive a player's minimum acceptable wage algebraically as the wage at which his total appraisal utility equals zero, rather than computing it from an independent formula.

#### Scenario: Reservation wage reflects a compensable step-down
- **WHEN** a player's sporting-axis penalty from a step-down is known
- **THEN** his reservation wage is exactly the wage that offsets that penalty and every other axis to zero net utility

### Requirement: A club pursues a player only when their standing fits
Before any other stage, the gate SHALL ask whether the club and the player fit by three facts that change slowly. When any one fails, the move SHALL NOT reach "may be privately scouted": the club keeps no file on him, and nothing later follows. The reason SHALL name the fact that failed.

**His level.** He would play in the division the club's first team plays in. The division SHALL be read against his level floor, never against the club's reputation, and for every buyer, however close its reputation is to his club's.

- For a purchase, a division less than a full refusal span under his floor SHALL pass. A division further under it SHALL pass only when the most the club could hold for his shirt reaches the wage at which, by his own appraisal, money alone carries him there. That wage SHALL rise the more of his career and his contract he still has to run, and SHALL fall toward what he earns late in a career or on an expiring deal.
- For a loan, which tables no wage of its own, any division under his floor SHALL fail. A player who has formally requested a transfer SHALL be exempt, and his own appraisal of the terms decides. A forced route SHALL be exempt.
- An unread division or an unread floor SHALL object to nothing.
- Whatever widens his floor SHALL widen who may pursue him, by the same amount: youth, months unsold, or a plan to step down.

**His wage.** The club could pay him.

- For a purchase, the most the club could hold for his shirt SHALL reach the wage he would ask to move. The ask SHALL come from the same appraisal that decides personal terms: the wage at which a move that changes nothing else about his football is worth making to him. Whatever pushes him to leave SHALL lower it and whatever holds him where he is SHALL raise it, continuously. No category of player SHALL be exempt from the comparison.
- What the club could hold for his shirt SHALL be the largest of three: its own level of pay for a player like him, stretched as far as it stretches for any signing; the room its wage structure gives one shirt; and its owner's envelope.
- For a loan, the club's share of the wage he already has SHALL be within what it can carry.

**Its market.** The club has a way into the market he is in: its reach into that market, from the corridor between the two countries and what the club itself knows of it, is at or above the market floor. A route forced by a release clause, a player who has formally asked to leave, and a player listed long enough to be resigned to it SHALL pass regardless, because in each his own side is reaching out.

Facts that change with events SHALL NOT decide whether a file may exist. Whether his club would release him and whether the fee is within reach SHALL cap how far the move goes, as the later stages already do.

#### Scenario: A club in a division he refuses keeps no file on him
- **WHEN** a 20-year-old squad player at a top-flight club, with four years on his contract, is a candidate for a club whose division sits a full refusal span under his floor and which could hold less for his shirt than he earns
- **THEN** the move does not reach "may be privately scouted", by any route, and the reason names his level

#### Scenario: A big name in a weak division is read by its division
- **WHEN** a club whose reputation is within one tier of a giant's plays in a division a full refusal span under the floor of that giant's key player, and cannot pay what carries him there
- **THEN** it does not pursue him, although its reputation alone would have let it

#### Scenario: Late in a career the floor is for sale to a club that can pay it
- **WHEN** a 33-year-old on an expiring deal and a 26-year-old with four years left share a floor a full refusal span above a division, and a club in that division could hold half as much again as either man's wage for one shirt
- **THEN** the club may pursue the 33-year-old
- **AND** it does not pursue the 26-year-old

#### Scenario: Months unsold widen who may pursue him
- **WHEN** a listed backup's floor has fallen, over months unsold, to within a refusal span of a club's division
- **THEN** his level no longer stops that club

#### Scenario: A club that cannot pay him
- **WHEN** a settled 27-year-old squad player earning 3.5M a year is read by a club in a division he would play in, whose own level pays a player like him 0.6M and which has 1M of room for one shirt
- **THEN** the move does not reach "may be privately scouted", and the reason names his wage

#### Scenario: A player pushed out asks less
- **WHEN** the same player has been listed by his club and left out for months, so that the wage he would ask to move has fallen under what that club could hold for his shirt
- **THEN** his wage no longer stops that club

#### Scenario: A club with no way into his market
- **WHEN** a player is read by a club in a country that neither imports from the country he plays in nor from his own, and that has no scout and no signing of its own there, so that its reach is under the market floor
- **THEN** the club keeps no file on him
- **AND** once he formally asks to leave, the market no longer stops that club

#### Scenario: A loan into a division under his floor
- **WHEN** a borrower whose club reputation is still top-flight after relegation now plays two divisions under an international's floor
- **THEN** the loan is not pursued, and the reason names his level

#### Scenario: A transfer request leaves a loan to him
- **WHEN** the same international has formally requested a transfer
- **THEN** his level does not stop the approach, and the borrower's division is weighed in his own appraisal of the terms

#### Scenario: What his club would do is not standing
- **WHEN** a club's standing fits a first-team regular whose club would not sell him
- **THEN** the club may keep a file on him, and the move goes no further until his club's position changes

### Requirement: A club is read one way by every route
The system SHALL read a buying club's side of any move from one account of the club, taken once for a pass: the division its first team plays in, its standing, what it could hold for one shirt, the fee it can reach, and the markets it knows. Every route and every stage SHALL use that account.

- The first team SHALL be the club's main squad.
- The fee it can reach SHALL be the gate's affordable ceiling. When the club's finance holds no transfer budget, every route SHALL fall back to the same figure, the budget on its transfer plan.
- No route SHALL read the club's standing, division, wage room or fee reach by a rule of its own.

#### Scenario: Two routes cannot disagree about the club
- **WHEN** the domestic approach and the foreign approach read the same club on the same day
- **THEN** both read the same standing, the same division, the same wage ceiling for a given player and the same fee reach

#### Scenario: A club with no budget on its books
- **WHEN** a club's finance holds no transfer budget and its transfer plan holds one
- **THEN** every route reads the plan's budget as the fee it can reach

### Requirement: Fee affordability gates negotiation
The system SHALL block a permanent-move negotiation unless the buyer can fund the fee for the player. Whether the buyer can fund his wage SHALL be read earlier, as part of standing fit. A player's availability SHALL NOT waive the wage comparison. It lowers what he asks.

#### Scenario: Fee ceiling blocks a permanent move
- **WHEN** a player's estimated value exceeds the buyer's affordable ceiling (1.40 times its transfer budget, floored at 25% of wage budget for emergency funding)
- **THEN** the permanent-move negotiation is blocked on fee grounds

#### Scenario: Availability lowers the ask instead of waiving the test
- **WHEN** a player his club has listed, who has been frozen out for months, is read by a club that could hold less for his shirt than he earns now but more than he would now ask
- **THEN** the wage side does not stop the move
- **AND** a club that could hold less than he would ask does not open talks for him, listed or not
