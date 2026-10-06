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

### Requirement: A player's level floor is the lowest division he will play in
A player SHALL carry a level floor on the league-reputation scale (0–10000), made of a standing and a tolerance.

The standing SHALL be the strongest of three claims:

- the division his observable ability places him at, on the same curve clubs' starter baselines are measured on
- his market reputation, carried further by senior international caps (full reach at about thirty caps)
- the division he plays in now, scaled by how much of a regular he is there, counted only once enough of his matches
  have been seen

The standing SHALL NOT exceed the reputation of the division he already plays in, when that is known.

The tolerance SHALL be one division's step-down for every player. It SHALL widen continuously with youth (most at
sixteen, nothing extra from twenty-three), with months unsold on the market, and with his own career plan to step down
a level. It SHALL never be less than one division's step-down, at any age.

How far a division sits under the floor SHALL read from zero at the floor to one at a full refusal span under it. It
SHALL read zero when either the division's reputation or the player's standing is unknown.

#### Scenario: An international does not play in the third tier
- **WHEN** a 27-year-old fifty-cap international whose ability starts a top-flight side has just been signed by a
  top-flight giant (8750) and not yet picked, and is measured against that country's second (5500) and third (3500)
  divisions
- **THEN** the second division sits at or above his floor and the third sits a full refusal span under it

#### Scenario: The strongest claim stands
- **WHEN** one player's ability reads lower than his name, and another's name reads lower than the top-flight
  division he starts nearly every match in
- **THEN** the first player's standing is his name, and the second's is that division

#### Scenario: The floor never sits above the division he already plays in
- **WHEN** a player whose name reads as top-flight plays in the second division
- **THEN** his standing is capped at the second division's reputation

#### Scenario: The tolerance widens with youth, months unsold and a plan to step down
- **WHEN** players aged 16, 19 and 23 with no market resignation and no plan are compared, and a 27-year-old who is
  fully resigned, or fully committed to stepping down, is compared with one who is neither
- **THEN** the tolerance falls from 16 to 19 to 23, is exactly one division's step-down from 23 onward, and is wider
  for the resigned and for the committed 27-year-old
- **AND** a season unsold lets the international above stop refusing the third division outright

#### Scenario: No view objects to nothing
- **WHEN** either the division's reputation or the player's standing is unknown
- **THEN** the division reads as not under his floor at all
