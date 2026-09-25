# core/club/player/transfer Specification

## Purpose
Owns a player's own transfer-market posture — big-stage pull, free-agent state, market-availability processing and the availability/block-reason diagnosis for a signed, market-exposed player.

## Requirements

### Requirement: Big-stage pull escalates through inclination, mood and formal request
A player's desire to move to a bigger competition SHALL be modeled as one continuous score with three tiers of consequence — a silent inclination the market can act on, a visible recurring mood event with a morale drag, and a formal transfer request — and escalation to a request SHALL require persistence of the itch or a denied move, not a period of unhappiness.

#### Scenario: A good player in a sub-elite league has gone unbought
- **WHEN** a player's big-stage pull has persisted for a season without an incoming bid, or a concrete move for him was blocked
- **THEN** the player's own transfer request is triggered even though he was not previously unhappy

### Requirement: A durable block-reason diagnosis explains a stalled availability
When the market fails to produce interest in a signed, market-available player, the player's transfer state SHALL retain a ranked reason explaining why, refreshed on every scan.

#### Scenario: Listed player draws no interest after weeks on the market
- **WHEN** a transfer-listed player has been scanned by the market repeatedly with no plausible buyer found
- **THEN** the block reason is recorded (ranked from a shallow "no plausible buyer" up to richer, closer-to-a-deal reasons) and refreshed on every scan, rather than leaving the absence of interest unexplained

### Requirement: A player answers a settlement offer from his own market prospects and his wish to play
Offered a mutual termination, a player SHALL name the least settlement he will accept. That figure is the wages left
on his contract, less:

- what he expects to earn elsewhere over the same span, from the wage his ability commands at the level his
  sights have fallen to, net of the time he expects to spend finding a club
- a premium for getting back to football, which grows with the time he has gone without first-team football, his
  ambition, and a standing transfer request or unhappiness

The figure SHALL NOT be negative, and it SHALL NOT depend on why he was listed.

#### Scenario: A starved player with a market asks for little
- **WHEN** a 25-year-old who has been frozen out for a season, holds a transfer request, and could earn nearly his current wage elsewhere is offered a settlement
- **THEN** the least he will accept is a small fraction of his remaining wages

#### Scenario: A veteran without a market holds out for his money
- **WHEN** a 33-year-old on a large wage, whose ability commands a fraction of it elsewhere and who shows little ambition, is offered a settlement
- **THEN** the least he will accept is close to all of his remaining wages

#### Scenario: Wanting to play lowers the price of leaving
- **WHEN** two players are otherwise identical, but one holds a transfer request and has gone a season without first-team football while the other is content
- **THEN** the player who wants out names the lower settlement
