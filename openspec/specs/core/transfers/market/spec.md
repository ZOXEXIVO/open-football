# core/transfers/market Specification

## Purpose
Owns market state and market geography: the per-country transfer-window calendar, the cross-country affinity that prices where a player could plausibly move, and the decay of an unbid asking price over time.

## Requirements

### Requirement: Transfer window calendars are country-specific and gate registration and negotiation separately
The system SHALL maintain a distinct transfer-window calendar per country (or group of countries sharing a football calendar) and SHALL allow formal negotiation to begin before a window opens and continue briefly after it closes, while restricting registration completion strictly to the open window. A country's calendar SHALL repeat every year, so that the next window to open, a window that crosses New Year, and the negotiation band before a window that opens on 1 January read correctly on every day of the year.

#### Scenario: Negotiation opens ahead of the formal window
- **WHEN** a window is due to open in fewer than 14 days
- **THEN** clubs may begin formal negotiation for a move even though the window has not yet opened

#### Scenario: Registration is refused outside the open window
- **WHEN** a negotiation reaches agreement but the transfer window is currently closed and no longer within its post-close negotiation grace period
- **THEN** the move cannot be registered until the next window opens

#### Scenario: The next window is known after the year's last one
- **WHEN** a country on the European calendar has shut its summer window and its January window is three months off
- **THEN** its next window reads as opening on 1 January of the next year

#### Scenario: Talks open in December for the January window
- **WHEN** it is 20 December in a country on the European calendar
- **THEN** clubs may negotiate for the January window, and nothing can be registered until it opens

#### Scenario: A window across New Year is open in January
- **WHEN** it is 15 January in a country whose off-season window runs from December to January
- **THEN** that window is open, and the market's weekly evaluation of its clubs carries on through January

### Requirement: Market affinity between countries reflects nationality, geography, and money-led exceptions
The system SHALL compute the plausibility of a player moving to a given country from a blend of nationality-corridor strength, a "shop window" effect for players already playing outside their home country, diaspora presence, and league-region structural priors, and SHALL allow a money-led override that bypasses the ordinary corridor when the destination has sufficient import capacity.

#### Scenario: A move to the player's own nationality is always fully plausible
- **WHEN** a club is considering signing a player who holds the buying country's own nationality
- **THEN** the affinity score for that move is 1.0

#### Scenario: Money-led override bypasses corridor strength but not import capacity
- **WHEN** a move is flagged as money-led (owner-benefactor signing) to a country with high import capacity
- **THEN** the ordinary nationality/geography corridor is bypassed, but the move is still capped by the destination country's import capacity

#### Scenario: Political route restrictions block specific country pairs
- **WHEN** a transfer or loan is attempted between a Russian club and a Ukrainian club on or after 2022-02-24
- **THEN** the move is blocked regardless of any other eligibility signal, since the system blocks specific country pairs when a real-world political restriction applies, evaluated symmetrically from the historically accurate date

### Requirement: An unbid market listing decays in asking price over time
The system SHALL reduce the asking price of a player listing that receives no bids, on a fixed weekly cadence, down to
a floor fraction of the listing's anchor price. The anchor is the price set when the player was listed, or the price
the seller last re-anchored it to.

#### Scenario: Listing decays toward its floor
- **WHEN** a player listing has gone seven days without a bid
- **THEN** its asking price decreases by 5%, continuing weekly until it reaches 60% of its anchor price, below which it does not decay further

#### Scenario: A re-anchored listing decays from its new anchor
- **WHEN** the seller re-anchors a listing to a lower price than it was first listed at
- **THEN** weekly decay continues from the new anchor, down to 60% of that anchor rather than 60% of the original listing price

### Requirement: A listing records its market exposure and the market's best named price
Every seller-listed permanent listing SHALL count the days it has been available while its country's transfer window
was open. Days while the window is shut SHALL NOT be counted. The listing SHALL also keep the highest fee from a bid
the seller rejected for price, for the life of the listing.

#### Scenario: Exposure counts only open-window days
- **WHEN** a listing stays available through thirty days of an open window and then sixty days of a closed one
- **THEN** its recorded exposure is thirty days

#### Scenario: A rejected bid is remembered
- **WHEN** the seller rejects two bids on a listing for price, of 1.5M and then 2M
- **THEN** the listing's best named price is 2M, and it stays 2M after both negotiations are cleared from the market

### Requirement: A listing made because he asked ends when he stops asking

A permanent listing SHALL record whether it exists because the player asked to go, through a formal request or a long
unhappiness, or because the club decided to sell him. His asking SHALL NOT set the club's own decision to sell.

When a player's request ends without a move, a listing that exists because he asked SHALL be withdrawn that day, and his
record SHALL show it. A live negotiation SHALL hold the withdrawal until it resolves. A listing the club decided on for
its own reasons SHALL NOT be withdrawn by this rule. If the club decides to sell a man already listed at his own request,
the listing SHALL become the club's, with the club's reason.

Once withdrawn he SHALL read as not available: no listing, no listed badge, no standing sale decision, and no push on
his appraisal for having been listed.

#### Scenario: He stopped asking
- **WHEN** a regular who asked to leave has withdrawn his request and has no live negotiation
- **THEN** his listing is withdrawn that day, and no buyer can pick him up from a broadcast afterwards

#### Scenario: A bid is on the table
- **WHEN** his request ends while a club's bid for him is pending
- **THEN** the listing stands until that negotiation resolves, and is then withdrawn if no move happened

#### Scenario: The club wants him gone for its own reasons
- **WHEN** his request ends while the club has decided to sell him over his wage
- **THEN** he stays listed, under the club's reason

#### Scenario: No badge is left behind
- **WHEN** a listing is withdrawn this way
- **THEN** he carries no listed badge and no sale flag, and the renewal pass may offer him terms again

### Requirement: A club's decision to sell reaches the market from any of its squads

A player the club has decided to sell SHALL get a permanent listing at the next open window whichever of the club's
squads he is registered with. The listing pass SHALL read the market's own listings, never the listed badge, to decide
that he has none.

His own request to leave and a grievance that has hardened SHALL reach the market the same way, and so SHALL the
club's decision whatever the checks that guard the listing pass's own reading of him say: a signing still inside his
club's patience, a man home from a loan in the open window, a loan candidacy the club no longer entertains, or an
unhappiness that has not hardened. Only a just-appointed head coach's review of the old regime's listings SHALL hold
the club's decision back.

#### Scenario: A reserve the club listed
- **WHEN** the club has listed a player registered with its reserve squad, who carries the listed badge and the club's sale decision but has no listing
- **THEN** the next open window gives him a listing a buyer can act on

#### Scenario: A loan candidacy the club no longer means
- **WHEN** a man back in the side carries an old loan candidacy the club no longer entertains, and the club has decided to sell him or he has asked to go
- **THEN** the next open window gives him a listing

#### Scenario: A loan the club still means goes first
- **WHEN** the club would still lend a man it has also decided to sell
- **THEN** he is offered on loan first, and the loan listing becomes a sale listing if nobody takes him for half a year

#### Scenario: Home from a loan and listed
- **WHEN** a club lists a man on the day he comes home from a loan, with the window open
- **THEN** he is listed in that window

#### Scenario: A signing the club has decided to sell
- **WHEN** the club decides to sell a man it signed this season before it has seen him play
- **THEN** he gets a listing, while the pass's own reading of him still waits out the club's patience

#### Scenario: A grumbling regular the club has listed
- **WHEN** the club lists a useful man who has been unhappy for a few weeks
- **THEN** he gets a listing
