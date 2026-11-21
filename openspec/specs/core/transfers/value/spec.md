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

### Requirement: Market value reads football played as a continuous record
The part of a player's market value that comes from his football SHALL be a continuous function of how much he has
played and how well, with no step between one count of appearances and the next.

- For the same quality of performance, more football SHALL never lower the value.
- A small sample SHALL be read with caution: one or two appearances, good or bad, SHALL move the value little.
- The record SHALL carry the previous season with a weight that fades as the current season fills, so that the
  value does not move on the day a season's statistics are archived.
- A player with no football at all in the period SHALL be valued below one who has played regularly at the same
  level, by an amount that grows with how long he has gone without.

#### Scenario: A debut
- **WHEN** a player with no appearances this season makes his first, with an ordinary rating
- **THEN** his market value does not fall

#### Scenario: One poor cameo
- **WHEN** a player's only appearance is a poor twenty minutes
- **THEN** his value moves by a small amount, not by a band

#### Scenario: The season turns
- **WHEN** a regular's season statistics are archived at the end of the season
- **THEN** his market value on the next day is within a small margin of the day before

#### Scenario: A year on the bench
- **WHEN** two players of the same level and age differ only in that one played thirty matches and the other none
- **THEN** the one who played is valued higher
