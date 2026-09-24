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
