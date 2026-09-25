# core/club/board/stranded Specification

## Purpose
The board's review of a player it listed for sale who got through a transfer window unsold. It decides what the club
does next: keep selling at a lower price, lend him out while paying part of his wage, or negotiate a mutual
termination. Nothing is released because of a calendar threshold.

## Requirements

### Requirement: A listing that survives a transfer window unsold is reviewed by its club's board

When a country's transfer window closes, the system SHALL hand every genuine seller-listed permanent listing that is
still available to its club's board for a stranded-listing review. A listing SHALL NOT be reviewed when:

- it was created only to back an unsolicited approach
- it is a loan or end-of-contract listing
- the player is out on loan
- a negotiation for the player is still pending or countered

Such a listing waits for the next window close.

#### Scenario: A listed player is still unsold when the window shuts
- **WHEN** a transfer window closes and a club's seller-listed permanent listing is still available with no live negotiation
- **THEN** that club's board reviews the listing that day

#### Scenario: A synthetic listing is not a stranded sale
- **WHEN** a window closes while a listing exists only to back an unsolicited bid
- **THEN** no stranded-listing review is made for that player

#### Scenario: A live bid defers the review
- **WHEN** a window closes while a bid for the listed player is still pending or countered
- **THEN** the listing is not reviewed at that close and is considered again at the next one

### Requirement: No listed player is released because of how long he has been listed

The system SHALL NOT end a listed player's contract because a fixed time has passed since he was listed. A listed
player SHALL leave his club only through a completed sale, a loan, a mutual termination both club and player agreed
to, or the natural expiry of his contract.

#### Scenario: Two years on the list without an agreed exit
- **WHEN** a player has been transfer-listed for more than two years, no buyer has met the club's price, and every settlement the board could offer is below the least he would accept
- **THEN** he remains under contract at his club, still listed, until a route clears or his contract expires

### Requirement: The board's resolve to end the stalemate is one continuous reading

The board SHALL read its resolve to end a stranded listing as one continuous value from 0 to 1. Resolve SHALL rise
with the market exposure the listing has had (days live while the window was open), with how heavily the player's
wage weighs on the club, and with how hard the player himself pushes to leave. The wage weight is his wage relative
to the club's wage budget, the club's wage pressure, and its cash need. The player's push is read from his own answer
to a settlement: the share of his remaining wages he is willing to give up to be let go. The reading SHALL NOT branch
on the reason he was listed, his league, his contract type, or whether he arrived on a fee.

#### Scenario: More exposure never lowers resolve
- **WHEN** the same club reviews the same listing at two window closes and every other input is unchanged
- **THEN** resolve at the later close is at least as high as at the earlier one

#### Scenario: A heavy earner presses harder than a cheap one
- **WHEN** two listings at the same club have had equal exposure and one player's wage is several times the other's
- **THEN** the board's resolve for the heavier earner is higher

#### Scenario: A player who would walk for nothing wears the board down sooner
- **WHEN** two listings at the same club have had equal exposure on equal wages, and one player would leave without any settlement while the other wants every penny he is owed
- **THEN** the board's resolve is higher for the player who would leave for nothing

#### Scenario: A listing made days before the close barely registers
- **WHEN** a player was listed a few days before the window closed
- **THEN** resolve is near zero and the review keeps him listed at close to his current asking price

### Requirement: Every review re-prices the listing from the board's own floor and the market's named price

Each review SHALL set a new asking anchor for a listing that stays on the market. The anchor SHALL NOT exceed the
current asking price. It SHALL NOT fall below the board's book floor, taken at the larger of the board's standing
write-off share and its resolve. As resolve rises, the anchor SHALL move toward the best bid the club rejected for
him. When nobody bid during the window, the market has refused the price asked, so the anchor SHALL fall toward zero
as resolve rises. After the review, the club SHALL accept a bid at or above the reviewed anchor rather than rejecting
it for price.

#### Scenario: A rejected bid names the price
- **WHEN** the club rejected a bid below the asking price during the window, and the board's resolve at the close is high
- **THEN** the new anchor sits at or near that bid, never below the board's book floor

#### Scenario: A window without a bid cuts the price
- **WHEN** nobody bid for the listed player through the window, and the board's resolve at the close is high
- **THEN** the new anchor is well below the previous asking price, never below the board's book floor

#### Scenario: A reviewed price is honoured
- **WHEN** after a review a buyer bids at or above the reviewed anchor
- **THEN** the seller does not reject the bid for price

### Requirement: A player with a career ahead of him is lent out with the club paying part of his wage

When the stranded player has loan runway (at least a year left on his contract and enough career left for a spell
elsewhere) and no settlement is worth more to the club than the loan, the review SHALL stage a loan. The share of his
wage the club keeps paying SHALL rise with the board's resolve. His sale listing SHALL stay live alongside the loan.

#### Scenario: A young stranded player goes out on loan
- **WHEN** a 24-year-old with three years left on his deal and a sale-market belief the board has not yet given up on is reviewed
- **THEN** a loan is staged for him with the club paying part of his wage, and his permanent listing remains available

#### Scenario: A near-expiry veteran has no loan route
- **WHEN** a 34-year-old with eight months left on his deal is reviewed
- **THEN** no loan is staged, and the review chooses between a settlement and keeping him listed

### Requirement: A mutual termination is agreed only when both the board and the player accept a settlement

The board SHALL offer a settlement only up to the most it will pay to stop carrying the player, meaning the wage it
saves, less:

- the part of his book value it is not yet willing to write off
- the resale it still believes in, which shrinks as resolve rises

That offer is also capped by what the club can pay as a lump sum. The player SHALL answer with the least he will
accept to leave. A termination SHALL happen only when the board's most is at least the player's least. The agreed
settlement SHALL lie between the two. The club SHALL choose the settlement over a loan or keeping him listed only when
it is worth more to the club.

#### Scenario: A frozen-out player with a market takes a pay-off
- **WHEN** a player on a heavy wage has been stranded through several windows, has no book value left, and could earn a comparable wage elsewhere
- **THEN** the club and player agree a mutual termination with a settlement below the wages remaining on his contract

#### Scenario: A veteran with no market keeps his contract
- **WHEN** the least a player will accept to leave is close to all his remaining wages, and that exceeds what the club can pay as a lump sum
- **THEN** no termination happens and he stays listed under contract

#### Scenario: A recent big-fee signing is not paid off
- **WHEN** the board still carries most of the fee it paid for the player and its resolve is low
- **THEN** the board makes no settlement offer at that review

### Requirement: A settled exit is a mutual termination with the negotiated money

When a settlement is agreed, the system SHALL:

- end the player's contract as a mutual termination ("released by mutual agreement" in his history)
- charge the club the agreed settlement, not a fixed severance formula
- retire all of his listing rows and every club's standing interest in him
- close the club's signing mandate for him as released

He then enters the free-agent pool through the normal sweep.

#### Scenario: The settlement is booked and the player is freed
- **WHEN** a settlement of a given amount is agreed at a window-close review
- **THEN** the player's contract is cleared with the mutual-termination reason, the club's expenses rise by exactly that amount, his listings are cancelled, and he is swept into the free-agent pool
