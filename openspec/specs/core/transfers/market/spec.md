# core/transfers/market Specification

## Purpose
Owns market state and market geography: the per-country transfer-window calendar, the cross-country affinity that prices where a player could plausibly move, and the decay of an unbid asking price over time.

## Requirements

### Requirement: Transfer window calendars are country-specific and gate registration and negotiation separately
The system SHALL maintain a distinct transfer-window calendar per country (or group of countries sharing a football calendar) and SHALL allow formal negotiation to begin before a window opens and continue briefly after it closes, while restricting registration completion strictly to the open window.

#### Scenario: Negotiation opens ahead of the formal window
- **WHEN** a window is due to open in fewer than 14 days
- **THEN** clubs may begin formal negotiation for a move even though the window has not yet opened

#### Scenario: Registration is refused outside the open window
- **WHEN** a negotiation reaches agreement but the transfer window is currently closed and no longer within its post-close negotiation grace period
- **THEN** the move cannot be registered until the next window opens

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
The system SHALL reduce the asking price of a player listing that receives no bids, on a fixed weekly cadence, down to a floor fraction of the original price.

#### Scenario: Listing decays toward its floor
- **WHEN** a player listing has gone seven days without a bid
- **THEN** its asking price decreases by 5%, continuing weekly until it reaches 60% of the original asking price, below which it does not decay further
