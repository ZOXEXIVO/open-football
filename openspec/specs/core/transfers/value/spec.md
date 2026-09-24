# core/transfers/value Specification

## Purpose
Prices what a player, a signing, and a wage are worth: the seller's asking figure, what a specific signing is worth to a specific buyer given who it displaces, and the ceiling a buyer can actually offer in wages.

## Requirements

### Requirement: An asking price defaults to a market-value calculation adjusted for listing and request status
The system SHALL price a player's asking figure from the club's own set ledger price when present, and otherwise from his computed market value discounted for being publicly listed or having requested a transfer, and adjusted by the seller's financial distress.

#### Scenario: Listed and transfer-requested discounts stack
- **WHEN** a player is both transfer-listed and has formally requested a move, and his club's ledger has not set an explicit asking price
- **THEN** his asking price is his base market value multiplied by 0.9 for being listed and by 0.85 for the transfer request

#### Scenario: A distressed seller asks under value, a solvent seller asks over
- **WHEN** the selling club's balance is negative
- **THEN** the asking price is discounted by the distress multiplier (0.9×) rather than inflated by the solvent-club premium (1.1×)

### Requirement: A signing's sporting value is judged against who it actually displaces
The system SHALL compute the sporting benefit of a signing relative to whichever player he actually takes minutes from — the promised incumbent at full commitment, or a blended deputy level scaled by the promised playing-time share for a partial role — rather than against the best player already in that position group.

#### Scenario: A bench-role signing is priced against the deputy, not the starter
- **WHEN** a candidate is being evaluated for a squad-cover role rather than the starting shirt
- **THEN** his contribution is computed relative to the club's own backup player's level, not the first-choice starter's level

#### Scenario: A signing who is not an upgrade on the man he displaces earns no value
- **WHEN** a candidate's believed ability does not exceed the level of the player whose minutes he would actually take
- **THEN** the deal's sporting benefit is zero rather than negative

### Requirement: A wage offer ceiling combines the buyer's own level, wage-budget headroom, and owner-subsidy envelope
The system SHALL cap the wage a club can offer at the greatest of its own reputation-implied wage stretched by a fixed factor, remaining room under the board's wage mandate net of any owner subsidy already counted there, or the club's remaining season allocation of an owner-subsidy envelope split across signing tiers.

#### Scenario: Owner subsidy is not double-counted
- **WHEN** a club's wage mandate already includes its owner's subsidy figure
- **THEN** the wage-headroom term used for the ceiling calculation has that subsidy amount subtracted back out before being combined with the separate owner-envelope term

#### Scenario: Concurrent negotiations reserve their share of the same tier envelope
- **WHEN** a club has multiple open negotiations in the same brief tier on the same day
- **THEN** each negotiation's ceiling reflects the tier envelope's remaining balance after reserving what the other open negotiations would draw if they completed
