# core/transfers/deal Specification

## Purpose
Prices and sequences one transaction once a move is plausible: what a bidding war does to the fee, what the deadline does to a buyer's willingness, and the fixed phases a permanent negotiation walks through before it closes.

## Requirements

### Requirement: Competing bidders raise the price and the seller's willingness to engage
The system SHALL require any bid for a player under multi-club interest to exceed the current highest rival offer by a minimum raise before it counts as a real improvement, and SHALL increase the seller's willingness to negotiate as the number of rival bidders grows, with diminishing returns.

#### Scenario: Minimum raise enforced against a rival's offer
- **WHEN** a club bids on a player who already has a live rival offer
- **THEN** the new bid must exceed the rival offer by at least 4% to be treated as a genuine raise

#### Scenario: Seller willingness saturates with rival count
- **WHEN** a fourth rival club enters bidding for the same player
- **THEN** the seller's willingness-to-engage bonus from bidder count does not increase further, having already saturated at three rivals

### Requirement: Deadline pressure inflates offers only for slots the buyer cannot walk away from
The system SHALL scale a buyer's willingness to pay above asking price during the closing days of a transfer window according to how pressure ramps toward the deadline, and SHALL apply that premium only in proportion to how critical the vacancy is to the buyer.

#### Scenario: Transformative-need slot pays full deadline premium
- **WHEN** a buyer is filling a transformative (top-tier) need on the last day of the transfer window
- **THEN** it may pay up to 20% over asking price purely from deadline pressure

#### Scenario: Cover slot converts to a loan search instead of overpaying
- **WHEN** a buyer is filling a mere squad-cover need during deadline week
- **THEN** no deadline premium is applied and the club instead searches for a loan alternative

### Requirement: Negotiation proceeds through fixed sequential phases with an overall expiry
The system SHALL run a permanent-transfer negotiation through initial approach, club fee negotiation (up to a bounded number of rounds), personal-terms discussion, and medical/finalization, in that order, and SHALL expire the whole negotiation a fixed number of days after it was created if not resolved.

#### Scenario: Negotiation expires unresolved
- **WHEN** a negotiation has neither been accepted nor rejected by 45 days after its creation
- **THEN** its status becomes Expired

#### Scenario: A fee round cap limits haggling
- **WHEN** club fee negotiation reaches its maximum number of rounds without agreement
- **THEN** the negotiation moves toward rejection rather than continuing to haggle indefinitely

### Requirement: Opening wage offer is anchored and does not compound across rounds
The system SHALL fix the buyer's opening wage offer at the value used in the first personal-terms round and SHALL evaluate later wage rounds against that same anchor rather than compounding successive raises against each other.

#### Scenario: Wage anchor prevents compounding
- **WHEN** a negotiation goes through two wage rounds each individually raising the offer by 30%
- **THEN** the second round's raise is measured against the original anchor wage, not against the already-raised figure from the first round
