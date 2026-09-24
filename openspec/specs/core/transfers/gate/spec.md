# core/transfers/gate Specification

## Purpose
Decides whether a prospective move may happen at all — the staged plausibility model, the player's own appraisal of an offer, and the squad-fit and quota checks a buyer must clear.

## Requirements

### Requirement: Staged eligibility gate for every move
The system SHALL evaluate every prospective transfer or loan through an ordered sequence of eligibility stages (from "may be privately scouted" through "may be shortlisted," "may show public interest," "may open negotiation," "may agree personal terms," to "may complete") and SHALL only allow a downstream action once the move has cleared every stage up to and including the one that action requires.

#### Scenario: Move blocked before negotiation
- **WHEN** a club wants to open formal negotiations for a player who has not cleared the negotiation-eligibility stage
- **THEN** the system refuses to create a negotiation and reports the blocking reason instead

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

### Requirement: Wage and fee affordability gate negotiation
The system SHALL block a permanent-move negotiation unless the buyer can plausibly fund a credible wage and fee for the player, using the player's availability strength to determine how strict that affordability check is.

#### Scenario: Fee ceiling blocks a permanent move
- **WHEN** a player's estimated value exceeds the buyer's affordable ceiling (1.40 times its transfer budget, floored at 25% of wage budget for emergency funding)
- **THEN** the permanent-move negotiation is blocked on fee grounds

#### Scenario: Real or forced availability waives the wage floor
- **WHEN** a player is graded Real or Forced availability
- **THEN** the negotiation is not blocked by the ordinary wage-floor requirement that would otherwise apply

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
The system SHALL evaluate whether a player accepts a move as a single utility score summed across weighted axes (money, sporting trajectory, promised role, destination prestige, home pull, pressure to leave, attachment to the current club, and personal circumstance/memory), and SHALL accept the move only when that utility, plus a fixed per-negotiation random disposition, exceeds zero.

#### Scenario: A raised offer that reaches the player's number is guaranteed accepted
- **WHEN** a club raises an offer so that the computed utility plus the player's disposition exceeds zero
- **THEN** the player accepts; the acceptance is not re-rolled at the higher offer

#### Scenario: A sporting step-down needs a compensating wage
- **WHEN** a move represents a sporting step-down for an ambitious player
- **THEN** the money axis must be raised enough to offset the sporting-axis penalty before total utility can turn positive

### Requirement: Reservation wage is derived, not separately authored
The system SHALL derive a player's minimum acceptable wage algebraically as the wage at which his total appraisal utility equals zero, rather than computing it from an independent formula.

#### Scenario: Reservation wage reflects a compensable step-down
- **WHEN** a player's sporting-axis penalty from a step-down is known
- **THEN** his reservation wage is exactly the wage that offsets that penalty and every other axis to zero net utility
