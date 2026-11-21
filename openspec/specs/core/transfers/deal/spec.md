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

### Requirement: A seller answers knowing how ready it is to replace the player
A club approached for a player it has not chosen to sell SHALL form one reading of how ready it is to replace him,
between not at all and fully, from:
- a player already at the club, in his shirt, perceived at or near his level;
- names the recruitment desk holds behind him that the offered fee would buy and that would plausibly come;
- the days left in the transfer window to complete a signing.

The seller's price SHALL rise continuously as readiness falls and as the player's importance to the club rises, for
buyers at home and abroad alike. When readiness is low, the player is a first choice and too few days remain to sign
a replacement, the seller SHALL decline the approach for this window unless the offer stands far above his value, a
release clause is met, or the player himself is forcing the move. A declined approach SHALL be remembered, so that
the buyer can return in the next window and the seller's desk prepares for it.

A candidate SHALL count toward readiness only if he plays the same shirt and stands near the player's level.

#### Scenario: An heir in the building
- **WHEN** a club is offered fair value for a starter whose planned heir is perceived at his level
- **THEN** it negotiates as a willing seller

#### Scenario: Nobody behind him, three weeks left
- **WHEN** a club with no heir and no names behind a starter is approached with three weeks of the window left
- **THEN** it asks for clearly more than his value

#### Scenario: Deadline day
- **WHEN** the same club is approached on the last day of the window with an offer at his value
- **THEN** it declines for this window

#### Scenario: An offer it cannot refuse
- **WHEN** the offer on the last day is far above his value
- **THEN** the club may accept

#### Scenario: A cheap full-back is not a successor to a centre-back
- **WHEN** the only name the desk holds in the same position group plays a different shirt at a much lower level
- **THEN** the club's readiness to replace the starter is not raised by him

### Requirement: A buyer that has just banked a fee is asked for more
A selling club SHALL raise its asking price to a buyer in proportion to how large the fees that buyer has received
this window are against the price asked, with diminishing effect, and more so when the buyer is replacing a player
it has just sold. A buyer with no fresh income SHALL be asked the ordinary price.

#### Scenario: Flush with a record sale
- **WHEN** a club that received 100M last week asks about a 30M player
- **THEN** it is quoted more than a club with no sales would be

#### Scenario: A small sale
- **WHEN** a club that received 2M asks about the same player
- **THEN** it is quoted the ordinary price
